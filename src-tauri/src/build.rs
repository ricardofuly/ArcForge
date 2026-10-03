//! Build the project's Editor target before launching, without opening an IDE.
use crate::config::EngineInstall;
use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{atomic::{AtomicBool, Ordering}, mpsc};
use tauri::{Emitter, Window};
use walkdir::WalkDir;

static LAUNCH_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct LaunchGuard;
impl LaunchGuard {
    pub fn acquire() -> Result<Self> {
        LAUNCH_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| anyhow!("Já existe uma compilação/abertura em andamento. Aguarde."))?;
        Ok(Self)
    }
}
impl Drop for LaunchGuard {
    fn drop(&mut self) { LAUNCH_ACTIVE.store(false, Ordering::SeqCst); }
}

pub fn needs_build(project: &Path) -> Result<bool> {
    let raw = std::fs::read_to_string(project).context("Não foi possível ler o .uproject")?;
    let json: serde_json::Value = serde_json::from_str(&raw).context(".uproject inválido")?;
    let root = project.parent().context("Projeto sem diretório")?;
    if root.join("Source").is_dir()
        || json.get("Modules").and_then(|v| v.as_array()).is_some_and(|m| !m.is_empty()) {
        return Ok(true);
    }
    let plugins = json.get("Plugins").and_then(|v| v.as_array());
    for entry in WalkDir::new(root.join("Plugins")).into_iter().filter_entry(|e| {
        !matches!(e.file_name().to_str(), Some("Binaries" | "Intermediate" | "Content" | ".git"))
    }).filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|ext| ext != "uplugin") { continue; }
        let name = entry.path().file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let enabled = plugins.and_then(|list| list.iter().find(|p| {
            p.get("Name").and_then(|v| v.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(name))
        })).and_then(|p| p.get("Enabled")).and_then(|v| v.as_bool());
        if enabled == Some(false) || !entry.path().parent().is_some_and(|p| p.join("Source").is_dir()) { continue; }
        let descriptor: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(entry.path())?)
            .with_context(|| format!("Plugin inválido: {}", entry.path().display()))?;
        if enabled.is_none() && descriptor.get("EnabledByDefault").and_then(|v| v.as_bool()) == Some(false) { continue; }
        if descriptor.get("Modules").and_then(|v| v.as_array()).is_some_and(|m| !m.is_empty()) { return Ok(true); }
    }
    Ok(false)
}

fn editor_target(project: &Path) -> Result<String> {
    let source = project.parent().context("Projeto sem diretório")?.join("Source");
    let mut targets = Vec::new();
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() { continue; }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".Target.cs") { continue; }
        let raw = std::fs::read_to_string(entry.path())?;
        // Ignore line comments; accept the usual whitespace around TargetType.Editor.
        let compact: String = raw.lines().map(|l| l.split("//").next().unwrap_or(""))
            .collect::<String>().chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("Type=TargetType.Editor") {
            targets.push(name.trim_end_matches(".Target.cs").to_string());
        }
    }
    targets.sort();
    targets.dedup();
    match targets.as_slice() {
        [target] => Ok(target.clone()),
        [] => Ok(format!("{}Editor", project.file_stem().and_then(|s| s.to_str()).context("Nome de projeto inválido")?)),
        _ => Err(anyhow!("Mais de um target Editor encontrado: {}. Defina um único target Editor para este projeto.", targets.join(", "))),
    }
}

/// cmd.exe and batch files expand shell syntax even inside quoted arguments.
#[cfg(any(windows, test))]
fn validate_batch_argument(value: &str) -> Result<()> {
    if value.chars().any(|c| matches!(c, '"' | '%' | '!' | '&' | '|' | '<' | '>' | '^' | '\r' | '\n')) {
        return Err(anyhow!("O caminho/target contém caracteres incompatíveis com Build.bat: use um caminho sem %, !, &, aspas ou operadores do shell."));
    }
    Ok(())
}

fn build_command(engine: &EngineInstall, project: &Path, target: &str) -> Result<Command> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let script = Path::new(&engine.path).join("Engine/Build/BatchFiles/Build.bat");
        if !script.is_file() { return Err(anyhow!("Build.bat não encontrado em {}", script.display())); }
        let script = script.to_str().context("Caminho da engine inválido")?;
        let project = project.to_str().context("Caminho do projeto inválido")?;
        for arg in [script, project, target] { validate_batch_argument(arg)?; }
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/d", "/s", "/c"]);
        cmd.raw_arg(format!("\"\"{script}\" {target} Win64 Development \"-Project={project}\" -WaitMutex -NoHotReloadFromIDE -NoEngineChanges -Progress\""));
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        Ok(cmd)
    }
    #[cfg(not(windows))]
    {
        let script = Path::new(&engine.path).join("Engine/Build/BatchFiles/Linux/Build.sh");
        if !script.is_file() { return Err(anyhow!("Build.sh não encontrado em {}", script.display())); }
        let mut cmd = Command::new("bash");
        cmd.arg(script).args([target, "Linux", "Development"])
            .arg(format!("-Project={}", project.display()))
            .args(["-WaitMutex", "-NoHotReloadFromIDE", "-NoEngineChanges", "-Progress"]);
        Ok(cmd)
    }
}

fn progress(line: &str) -> Option<f64> {
    if line.contains("@progress") {
        // UHT reports 100% for its own nested phase, before any C++ action runs.
        if !line.contains("Compiling C++") { return None; }
        let end = line.rfind('%')?;
        let number = line[..end].split_whitespace().last()?;
        return number.parse::<f64>().ok().map(|n| n.clamp(0.0, 99.0));
    }
    let start = line.find('[')?;
    let end = line[start..].find(']')? + start;
    let (done, total) = line[start + 1..end].split_once('/')?;
    let done: f64 = done.trim().parse().ok()?;
    let total: f64 = total.trim().parse().ok()?;
    (total > 0.0).then(|| (done / total * 100.0).clamp(0.0, 99.0))
}

fn stream<R: std::io::Read + Send + 'static>(reader: R, tx: mpsc::Sender<String>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => { if tx.send(String::from_utf8_lossy(&bytes).trim_end().to_string()).is_err() { break; } }
                Err(e) => { let _ = tx.send(format!("Erro ao ler saída da compilação: {e}")); break; }
            }
        }
    })
}

pub fn compile(window: &Window, uproject: &str, engine: &EngineInstall) -> Result<()> {
    compile_with(uproject, engine, |stage, percent, line, log_path| {
        let _ = window.emit("project-build-progress", serde_json::json!({
            "uproject_path": uproject, "stage": stage, "percent": percent,
            "line": line, "log_path": log_path.to_string_lossy()
        }));
    })
}

fn compile_with(uproject: &str, engine: &EngineInstall, mut report: impl FnMut(&str, Option<f64>, &str, &Path)) -> Result<()> {
    let project = Path::new(uproject);
    if !needs_build(project)? { return Ok(()); }
    let log_dir = project.parent().context("Projeto sem diretório")?.join("Saved/Logs");
    std::fs::create_dir_all(&log_dir).context("Não foi possível criar a pasta de logs")?;
    let log_path = log_dir.join(format!("LauncherBuild-{}.log", uuid::Uuid::new_v4()));
    let mut log = std::fs::File::create(&log_path).context("Não foi possível criar o log")?;
    let mut emit = |stage: &str, percent: Option<f64>, line: &str| report(stage, percent, line, &log_path);
    emit("building", None, "Preparando compilação Development Editor…");
    let result = (|| -> Result<()> {
        let target = editor_target(project)?;
        if !target.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(anyhow!("Nome de target inválido: {target}"));
        }
        let mut cmd = build_command(engine, project, &target)?;
        let header = format!("Engine: {}\nProjeto: {}\nTarget: {} Development Editor", engine.path, uproject, target);
        writeln!(log, "{header}")?;
        emit("building", None, &header);
        let mut child = cmd.current_dir(&engine.path).stdin(Stdio::null())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            .context("Não foi possível iniciar o UnrealBuildTool")?;
        let (tx, rx) = mpsc::channel();
        let stdout = stream(child.stdout.take().context("stdout indisponível")?, tx.clone());
        let stderr = stream(child.stderr.take().context("stderr indisponível")?, tx.clone());
        drop(tx);
        let mut percent: Option<f64> = None;
        let mut log_error = None;
        for line in rx {
            if let Some(value) = progress(&line) { percent = Some(percent.unwrap_or(0.0).max(value)); }
            if let Err(e) = writeln!(log, "{line}") { log_error = Some(e); }
            emit("building", percent, &line);
        }
        let status = child.wait().context("Erro aguardando compilação")?;
        let _ = stdout.join();
        let _ = stderr.join();
        if !status.success() {
            return Err(anyhow!("Compilação falhou (código {:?}). Verifique os erros no log. No Windows, instale o MSVC C++ e o Windows SDK compatíveis com a engine. Feche o editor se ele estiver usando os módulos do projeto.", status.code()));
        }
        if let Some(e) = log_error { return Err(anyhow!("Não foi possível salvar o log: {e}")); }
        log.flush()?;
        Ok(())
    })();
    match result {
        Ok(()) => { emit("compiled", Some(100.0), "Compilação concluída. Iniciando editor…"); Ok(()) }
        Err(e) => {
            let message = format!("{e:#}\nLog: {}", log_path.display());
            let _ = writeln!(log, "{message}");
            emit("failed", None, &message);
            Err(anyhow!(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_guard_blocks_duplicates_and_releases_on_drop() {
        let guard = LaunchGuard::acquire().unwrap();
        assert!(LaunchGuard::acquire().is_err());
        drop(guard);
        assert!(LaunchGuard::acquire().is_ok());
    }
    #[test]
    fn parses_ubt_progress() {
        assert_eq!(progress("[3/12] Compile Foo.cpp"), Some(25.0));
        assert_eq!(progress("@progress 'Compiling C++' 40%"), Some(40.0));
        assert_eq!(progress("@progress 'Generating code...' 100%"), None);
        assert_eq!(progress("[12/12] Link"), Some(99.0));
        assert_eq!(progress("[1/0] Invalid"), None);
        assert_eq!(progress("error C2039: foo"), None);
    }
    #[test]
    fn batch_arguments_allow_spaces_and_unicode_but_block_expansion() {
        assert!(validate_batch_argument("C:\\Projetos de João\\Game.uproject").is_ok());
        for arg in ["%PATH%", "a!b", "a&b", "a\"b", "a\nb"] { assert!(validate_batch_argument(arg).is_err()); }
    }

    fn fixture() -> (std::path::PathBuf, std::path::PathBuf, EngineInstall) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/build-tests")
            .join(format!("Projeto de João {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Source")).unwrap();
        let project = root.join("Game.uproject");
        std::fs::write(&project, "{\"Modules\":[{\"Name\":\"Game\"}]}").unwrap();
        let engine_root = root.join("Engine com espaços");
        std::fs::create_dir_all(engine_root.join("Engine/Build/BatchFiles/Linux")).unwrap();
        let engine = EngineInstall {
            id: "test".into(), version: "5.8.0".into(), path: engine_root.to_string_lossy().into(),
            editor_binary: "unused".into(), is_source_build: false, label: "test".into()
        };
        (root, project, engine)
    }

    #[test]
    fn discovers_custom_editor_target_and_rejects_ambiguity() {
        let (root, project, _) = fixture();
        std::fs::write(root.join("Source/StudioEditor.Target.cs"), "// Type=TargetType.Editor\n Type = TargetType.Editor;").unwrap();
        assert_eq!(editor_target(&project).unwrap(), "StudioEditor");
        std::fs::write(root.join("Source/Other.Target.cs"), "Type=TargetType.Editor;").unwrap();
        assert!(editor_target(&project).is_err());
    }

    #[test]
    fn blueprint_skips_build_and_invalid_descriptor_fails() {
        let (root, project, engine) = fixture();
        let bp_dir = root.join("Blueprint");
        std::fs::create_dir_all(&bp_dir).unwrap();
        let bp = bp_dir.join("BP.uproject");
        std::fs::write(&bp, "{}").unwrap();
        assert!(!needs_build(&bp).unwrap());
        compile_with(bp.to_str().unwrap(), &engine, |_, _, _, _| panic!("Blueprint should skip compilation")).unwrap();
        std::fs::write(&project, "broken json").unwrap();
        assert!(needs_build(&project).is_err());
    }

    #[test]
    fn enabled_local_cpp_plugin_requires_build_but_disabled_plugin_does_not() {
        let (root, _, _) = fixture();
        let bp = root.join("BP");
        let plugin = bp.join("Plugins/Local");
        std::fs::create_dir_all(plugin.join("Source")).unwrap();
        std::fs::write(plugin.join("Local.uplugin"), "{\"Modules\":[{\"Name\":\"Local\"}]}").unwrap();
        let project = bp.join("BP.uproject");
        std::fs::write(&project, "{}").unwrap();
        assert!(needs_build(&project).unwrap());
        std::fs::write(&project, "{\"Plugins\":[{\"Name\":\"Local\",\"Enabled\":false}]}").unwrap();
        assert!(!needs_build(&project).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn batch_build_streams_both_pipes_preserves_log_and_propagates_failure() {
        let (_, project, engine) = fixture();
        let script = Path::new(&engine.path).join("Engine/Build/BatchFiles/Build.bat");
        for code in [0, 7] {
            std::fs::write(&script, format!("@echo off\r\necho Args: %*\r\necho [1/2] Compile\r\nfor /L %%i in (1,1,1500) do @echo stderr-line-%%i 1>&2\r\necho [2/2] Link\r\nexit /b {code}\r\n")).unwrap();
            let mut stages = Vec::new();
            let mut path = std::path::PathBuf::new();
            let result = compile_with(project.to_str().unwrap(), &engine, |stage, _, _, log| {
                stages.push(stage.to_string()); path = log.to_path_buf();
            });
            assert_eq!(result.is_ok(), code == 0);
            assert_eq!(stages.last().unwrap(), if code == 0 { "compiled" } else { "failed" });
            let log = std::fs::read_to_string(path).unwrap();
            assert!(log.contains("stderr-line-1500"));
            assert!(log.contains("-Project="));
            assert!(log.contains("Win64 Development"));
            assert!(log.contains("[2/2] Link"));
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Requires UNREAL_TEST_ENGINE, MSVC and Windows SDK; compiles a real C++ module"]
    fn real_unreal_editor_build() {
        let engine_path = std::env::var("UNREAL_TEST_ENGINE").expect("Set UNREAL_TEST_ENGINE to an installed Unreal Engine root");
        let actual_engine = crate::engine::detect_engine(&engine_path, false).unwrap();
        let (_, _, scaffold_engine) = fixture();
        // UBT still limits generated action paths to 260 characters on Windows.
        let root = std::env::temp_dir().join(format!("UL {}", &uuid::Uuid::new_v4().to_string()[..8]));
        let project = crate::project::create_unreal_project("LauncherSmoke", root.to_str().unwrap(), &scaffold_engine, true, "blank", None).unwrap();
        let mut log_path = std::path::PathBuf::new();
        compile_with(&project.uproject_path, &actual_engine, |_, _, line, log| {
            println!("{line}"); log_path = log.to_path_buf();
        }).expect("Real C++ compilation should succeed");
        let binaries = Path::new(&project.project_dir).join("Binaries/Win64");
        assert!(std::fs::read_dir(binaries).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "dll")));
        let cpp = Path::new(&project.project_dir).join("Source/LauncherSmoke/LauncherSmoke.cpp");
        std::fs::write(cpp, "#error LAUNCHER_EXPECTED_BUILD_ERROR\n").unwrap();
        assert!(compile_with(&project.uproject_path, &actual_engine, |_, _, line, log| {
            println!("{line}"); log_path = log.to_path_buf();
        }).is_err());
        assert!(std::fs::read_to_string(&log_path).unwrap().contains("LAUNCHER_EXPECTED_BUILD_ERROR"));
    }
}
