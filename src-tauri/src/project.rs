use crate::config::EngineInstall;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
struct UProjectFile {
    #[serde(rename = "EngineAssociation")]
    engine_association: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnrealProject {
    pub name: String,
    pub uproject_path: String,
    pub project_dir: String,
    pub has_source: bool,
    pub thumbnail_path: Option<String>,
    pub engine_association: Option<String>,
    pub matched_engine_id: Option<String>,
    #[serde(default)]
    pub rhi_mode: Option<String>,
}

/// Varre um diretório em busca de arquivos .uproject, até uma profundidade razoável,
/// pulando pastas pesadas que nunca contêm um .uproject na raiz do projeto (Intermediate, Saved, etc).
pub fn scan_directory_for_projects(
    root: &str,
    engines: &[EngineInstall],
    excluded: &[String],
) -> Vec<UnrealProject> {
    let mut results = Vec::new();

    let walker = WalkDir::new(root)
        .max_depth(6)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "Intermediate"
                    | "Saved"
                    | "Binaries"
                    | "Build"
                    | "DerivedDataCache"
                    | ".git"
                    | ".vs"
                    | ".idea"
                    | "node_modules"
                    | "target"
                    | ".gemini"
            )
        });


    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "uproject").unwrap_or(false) {
            let uproject_path = path.to_string_lossy().to_string();
            if excluded.contains(&uproject_path) {
                continue;
            }

            let project_dir = match path.parent() {
                Some(p) => p,
                None => continue,
            };

            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Sem nome".to_string());

            let has_source = project_dir.join("Source").is_dir();

            let engine_association = std::fs::read_to_string(path)
                .ok()
                .and_then(|raw| serde_json::from_str::<UProjectFile>(&raw).ok())
                .and_then(|u| u.engine_association);

            let matched_engine_id = engine_association.as_ref().and_then(|assoc| {
                engines.iter().find(|e| {
                    crate::engine::short_version(&e.version) == *assoc
                        || &e.version == assoc
                        || e.id == *assoc
                }).map(|e| e.id.clone())
            });

            // A Unreal salva uma captura de tela automática do editor pra usar como thumbnail
            // no Project Browser nativo — reaproveitamos o mesmo arquivo aqui.
            let auto_screenshot = project_dir.join("Saved/AutoScreenshot.png");
            let thumbnail_path = auto_screenshot
                .is_file()
                .then(|| auto_screenshot.to_string_lossy().to_string());

            results.push(UnrealProject {
                name,
                uproject_path,
                project_dir: project_dir.to_string_lossy().to_string(),
                has_source,
                thumbnail_path,
                engine_association,
                matched_engine_id,
                rhi_mode: None,
            });
        }
    }

    results
}

/// Lança o editor da Unreal para um projeto específico, de forma desacoplada (não bloqueia o launcher),
/// opcionalmente aplicando argumentos de RHI / Shader Model como -sm5 ou -sm6.
pub fn launch_project(
    uproject_path: &str,
    engine: &EngineInstall,
    rhi_mode: Option<&str>,
) -> Result<()> {
    let uproject = Path::new(uproject_path);
    if !uproject.is_file() {
        return Err(anyhow!("arquivo .uproject não encontrado: {}", uproject_path));
    }
    let editor = Path::new(&engine.editor_binary);
    if !editor.is_file() {
        return Err(anyhow!(
            "binário do editor não encontrado para a engine {}: {}",
            engine.label,
            engine.editor_binary
        ));
    }

    let mut cmd = Command::new(editor);
    cmd.arg(uproject);

    if let Some(rhi) = rhi_mode {
        match rhi.to_lowercase().as_str() {
            "sm5" => {
                cmd.arg("-sm5");
            }
            "sm6" => {
                cmd.arg("-sm6");
            }
            _ => {}
        }
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| "falha ao iniciar o UnrealEditor")?;

    Ok(())
}

/// Informações de uma IDE detectada no sistema
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IdeInfo {
    pub id: String,
    pub name: String,
    pub runner: String,
    pub is_available: bool,
}

#[derive(Debug, Clone)]
pub struct IdeLaunchCommand {
    pub program: String,
    pub args: Vec<String>,
    pub runner_name: String,
}

/// IDEs suportadas pra abrir o código C++ de um projeto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ide {
    VsCode,
    Rider,
    CLion,
    VisualStudio,
}

impl Ide {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "code" => Ok(Ide::VsCode),
            "rider" => Ok(Ide::Rider),
            "clion" => Ok(Ide::CLion),
            "vs" | "visualstudio" => Ok(Ide::VisualStudio),
            other => Err(anyhow!("IDE desconhecida: {other}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Ide::VsCode => "code",
            Ide::Rider => "rider",
            Ide::CLion => "clion",
            Ide::VisualStudio => "vs",
        }
    }

    pub fn friendly_name(&self) -> &'static str {
        match self {
            Ide::VsCode => "VS Code",
            Ide::Rider => "Rider",
            Ide::CLion => "CLion",
            Ide::VisualStudio => "Visual Studio",
        }
    }

    pub fn resolve_launch_command(&self) -> Option<IdeLaunchCommand> {
        let home = dirs::home_dir();
        let is_in_flatpak = Path::new("/.flatpak-info").exists();
        let has_flatpak_cli = check_command_in_path("flatpak") || Path::new("/usr/bin/flatpak").is_file();

        match self {
            Ide::VsCode => {
                // 1. Binário nativo no PATH
                for bin in ["code", "code.cmd", "code.exe", "codium", "code-oss"] {
                    if check_command_in_path(bin) {
                        return Some(IdeLaunchCommand {
                            program: bin.to_string(),
                            args: Vec::new(),
                            runner_name: "Nativo".to_string(),
                        });
                    }
                }

                // 2. Flatpak (com.visualstudio.code, com.vscodium.codium, com.visualstudio.code.oss)
                let flatpak_ids = [
                    "com.visualstudio.code",
                    "com.vscodium.codium",
                    "com.visualstudio.code.oss",
                ];
                for id in flatpak_ids {
                    if is_flatpak_app_installed(id) {
                        if is_in_flatpak {
                            return Some(IdeLaunchCommand {
                                program: "flatpak-spawn".to_string(),
                                args: vec!["--host".to_string(), "flatpak".to_string(), "run".to_string(), id.to_string()],
                                runner_name: "Flatpak".to_string(),
                            });
                        } else if has_flatpak_cli {
                            return Some(IdeLaunchCommand {
                                program: "flatpak".to_string(),
                                args: vec!["run".to_string(), id.to_string()],
                                runner_name: "Flatpak".to_string(),
                            });
                        } else {
                            let exp = Path::new("/var/lib/flatpak/exports/bin").join(id);
                            if exp.is_file() {
                                return Some(IdeLaunchCommand {
                                    program: exp.to_string_lossy().to_string(),
                                    args: Vec::new(),
                                    runner_name: "Flatpak Export".to_string(),
                                });
                            }
                        }
                    }
                }

                // 3. Snap
                for snap_path in ["/snap/bin/code", "/var/lib/snapd/snap/bin/code"] {
                    if Path::new(snap_path).is_file() {
                        return Some(IdeLaunchCommand {
                            program: snap_path.to_string(),
                            args: Vec::new(),
                            runner_name: "Snap".to_string(),
                        });
                    }
                }

                // 4. Windows caminhos padrão
                if let Some(local_app) = dirs::data_local_dir() {
                    let win_code = local_app.join("Programs/Microsoft VS Code/Code.exe");
                    if win_code.is_file() {
                        return Some(IdeLaunchCommand {
                            program: win_code.to_string_lossy().to_string(),
                            args: Vec::new(),
                            runner_name: "Windows".to_string(),
                        });
                    }
                }
            }
            Ide::Rider => {
                // 1. Binário nativo no PATH
                for bin in ["rider", "rider.sh", "rider64.exe", "rider.exe"] {
                    if check_command_in_path(bin) {
                        return Some(IdeLaunchCommand {
                            program: bin.to_string(),
                            args: Vec::new(),
                            runner_name: "Nativo".to_string(),
                        });
                    }
                }

                // 2. Flatpak (com.jetbrains.Rider)
                let flatpak_id = "com.jetbrains.Rider";
                if is_flatpak_app_installed(flatpak_id) {
                    if is_in_flatpak {
                        return Some(IdeLaunchCommand {
                            program: "flatpak-spawn".to_string(),
                            args: vec!["--host".to_string(), "flatpak".to_string(), "run".to_string(), flatpak_id.to_string()],
                            runner_name: "Flatpak".to_string(),
                        });
                    } else if has_flatpak_cli {
                        return Some(IdeLaunchCommand {
                            program: "flatpak".to_string(),
                            args: vec!["run".to_string(), flatpak_id.to_string()],
                            runner_name: "Flatpak".to_string(),
                        });
                    } else {
                        let exp = Path::new("/var/lib/flatpak/exports/bin").join(flatpak_id);
                        if exp.is_file() {
                            return Some(IdeLaunchCommand {
                                program: exp.to_string_lossy().to_string(),
                                args: Vec::new(),
                                runner_name: "Flatpak Export".to_string(),
                            });
                        }
                    }
                }

                // 3. JetBrains Toolbox
                if let Some(ref h) = home {
                    let tb_script = h.join(".local/share/JetBrains/Toolbox/scripts/rider");
                    if tb_script.is_file() {
                        return Some(IdeLaunchCommand {
                            program: tb_script.to_string_lossy().to_string(),
                            args: Vec::new(),
                            runner_name: "JetBrains Toolbox".to_string(),
                        });
                    }
                }

                // 4. Snap
                for snap_path in ["/snap/bin/rider", "/var/lib/snapd/snap/bin/rider"] {
                    if Path::new(snap_path).is_file() {
                        return Some(IdeLaunchCommand {
                            program: snap_path.to_string(),
                            args: Vec::new(),
                            runner_name: "Snap".to_string(),
                        });
                    }
                }

                // 5. Windows caminhos padrão
                if let Some(local_app) = dirs::data_local_dir() {
                    let win_rider = local_app.join("Programs/Rider/bin/rider64.exe");
                    if win_rider.is_file() {
                        return Some(IdeLaunchCommand {
                            program: win_rider.to_string_lossy().to_string(),
                            args: Vec::new(),
                            runner_name: "Windows".to_string(),
                        });
                    }
                }
            }
            Ide::CLion => {
                // 1. Binário nativo no PATH
                for bin in ["clion", "clion.sh", "clion64.exe", "clion.exe"] {
                    if check_command_in_path(bin) {
                        return Some(IdeLaunchCommand {
                            program: bin.to_string(),
                            args: Vec::new(),
                            runner_name: "Nativo".to_string(),
                        });
                    }
                }

                // 2. Flatpak (com.jetbrains.CLion)
                let flatpak_id = "com.jetbrains.CLion";
                if is_flatpak_app_installed(flatpak_id) {
                    if is_in_flatpak {
                        return Some(IdeLaunchCommand {
                            program: "flatpak-spawn".to_string(),
                            args: vec!["--host".to_string(), "flatpak".to_string(), "run".to_string(), flatpak_id.to_string()],
                            runner_name: "Flatpak".to_string(),
                        });
                    } else if has_flatpak_cli {
                        return Some(IdeLaunchCommand {
                            program: "flatpak".to_string(),
                            args: vec!["run".to_string(), flatpak_id.to_string()],
                            runner_name: "Flatpak".to_string(),
                        });
                    } else {
                        let exp = Path::new("/var/lib/flatpak/exports/bin").join(flatpak_id);
                        if exp.is_file() {
                            return Some(IdeLaunchCommand {
                                program: exp.to_string_lossy().to_string(),
                                args: Vec::new(),
                                runner_name: "Flatpak Export".to_string(),
                            });
                        }
                    }
                }

                // 3. JetBrains Toolbox
                if let Some(ref h) = home {
                    let tb_script = h.join(".local/share/JetBrains/Toolbox/scripts/clion");
                    if tb_script.is_file() {
                        return Some(IdeLaunchCommand {
                            program: tb_script.to_string_lossy().to_string(),
                            args: Vec::new(),
                            runner_name: "JetBrains Toolbox".to_string(),
                        });
                    }
                }

                // 4. Snap
                for snap_path in ["/snap/bin/clion", "/var/lib/snapd/snap/bin/clion"] {
                    if Path::new(snap_path).is_file() {
                        return Some(IdeLaunchCommand {
                            program: snap_path.to_string(),
                            args: Vec::new(),
                            runner_name: "Snap".to_string(),
                        });
                    }
                }

                // 5. Windows caminhos padrão
                if let Some(local_app) = dirs::data_local_dir() {
                    let win_clion = local_app.join("Programs/CLion/bin/clion64.exe");
                    if win_clion.is_file() {
                        return Some(IdeLaunchCommand {
                            program: win_clion.to_string_lossy().to_string(),
                            args: Vec::new(),
                            runner_name: "Windows".to_string(),
                        });
                    }
                }
            }
            Ide::VisualStudio => {
                // 1. Binário no PATH
                for bin in ["devenv", "devenv.exe", "devenv.com"] {
                    if check_command_in_path(bin) {
                        return Some(IdeLaunchCommand {
                            program: bin.to_string(),
                            args: Vec::new(),
                            runner_name: "Nativo".to_string(),
                        });
                    }
                }

                // 2. Instalações comuns do Visual Studio no Windows
                let vs_paths = [
                    "C:/Program Files/Microsoft Visual Studio/2022/Community/Common7/IDE/devenv.exe",
                    "C:/Program Files/Microsoft Visual Studio/2022/Professional/Common7/IDE/devenv.exe",
                    "C:/Program Files/Microsoft Visual Studio/2022/Enterprise/Common7/IDE/devenv.exe",
                    "C:/Program Files (x86)/Microsoft Visual Studio/2019/Community/Common7/IDE/devenv.exe",
                    "C:/Program Files (x86)/Microsoft Visual Studio/2019/Professional/Common7/IDE/devenv.exe",
                    "C:/Program Files (x86)/Microsoft Visual Studio/2019/Enterprise/Common7/IDE/devenv.exe",
                ];
                for path in vs_paths {
                    if Path::new(path).is_file() {
                        return Some(IdeLaunchCommand {
                            program: path.to_string(),
                            args: Vec::new(),
                            runner_name: "Windows".to_string(),
                        });
                    }
                }
            }
        }

        None
    }
}

fn check_command_in_path(cmd: &str) -> bool {
    if cmd.contains('/') || cmd.contains('\\') {
        return Path::new(cmd).is_file();
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let full = dir.join(cmd);
            if full.is_file() {
                return true;
            }
            #[cfg(target_os = "windows")]
            {
                if dir.join(format!("{}.exe", cmd)).is_file() || dir.join(format!("{}.cmd", cmd)).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

fn is_flatpak_app_installed(app_id: &str) -> bool {
    if Path::new("/var/lib/flatpak/app").join(app_id).is_dir() {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        if home.join(".local/share/flatpak/app").join(app_id).is_dir() {
            return true;
        }
    }
    if Path::new("/var/lib/flatpak/exports/bin").join(app_id).is_file() {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        if home.join(".local/share/flatpak/exports/bin").join(app_id).is_file() {
            return true;
        }
    }
    false
}

/// Retorna a lista de IDEs conhecidas e se foram detectadas no sistema.
pub fn list_detected_ides() -> Vec<IdeInfo> {
    let all = [Ide::VsCode, Ide::Rider, Ide::CLion, Ide::VisualStudio];
    all.iter()
        .map(|ide| {
            let cmd = ide.resolve_launch_command();
            IdeInfo {
                id: ide.as_str().to_string(),
                name: ide.friendly_name().to_string(),
                runner: cmd.as_ref().map(|c| c.runner_name.clone()).unwrap_or_default(),
                is_available: cmd.is_some(),
            }
        })
        .collect()
}

/// Abre a pasta do projeto na IDE escolhida. Pro VS Code, prioriza um arquivo
/// `.code-workspace` na raiz do projeto (gerado pelo UBT via "Generate Project Files"),
/// já que ele configura include paths e build tasks corretamente; sem isso, cai pra
/// abrir a pasta crua. Para o Rider e Visual Studio, prioriza o `.sln` ou `.uproject`.
pub fn open_in_ide(project_dir: &str, ide: Ide) -> Result<()> {
    let dir = Path::new(project_dir);
    if !dir.is_dir() {
        return Err(anyhow!("pasta do projeto não encontrada: {project_dir}"));
    }

    let target: std::path::PathBuf = if ide == Ide::VsCode {
        std::fs::read_dir(dir)
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|e| {
                    let p = e.path();
                    (p.extension().map(|ext| ext == "code-workspace").unwrap_or(false)).then_some(p)
                })
            })
            .unwrap_or_else(|| dir.to_path_buf())
    } else if ide == Ide::Rider || ide == Ide::VisualStudio {
        std::fs::read_dir(dir)
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|e| {
                    let p = e.path();
                    (p.extension().map(|ext| ext == "sln" || ext == "uproject").unwrap_or(false)).then_some(p)
                })
            })
            .unwrap_or_else(|| dir.to_path_buf())
    } else {
        dir.to_path_buf()
    };

    let cmd = ide.resolve_launch_command().ok_or_else(|| {
        anyhow!(
            "não encontrei o {} instalado no sistema. Verifique se está instalado e no PATH do sistema.",
            ide.friendly_name()
        )
    })?;

    let mut process = Command::new(&cmd.program);
    for arg in &cmd.args {
        process.arg(arg);
    }
    process.arg(&target);
    process.stdin(Stdio::null());
    process.stdout(Stdio::null());
    process.stderr(Stdio::null());

    process.spawn().with_context(|| {
        format!(
            "falha ao executar {} ({}) via '{}'",
            ide.friendly_name(),
            cmd.runner_name,
            cmd.program
        )
    })?;

    Ok(())
}

/// Cria um novo projeto Unreal Engine (Blueprint ou C++) diretamente no disco,
/// sem precisar inicializar o editor, utilizando os templates oficiais da engine ou scaffolding limpo.
pub fn create_unreal_project(
    name: &str,
    parent_dir: &str,
    engine: &EngineInstall,
    is_cpp: bool,
    template_key: &str,
    rhi_target: Option<&str>,
) -> Result<UnrealProject> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("o nome do projeto não pode ser vazio"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(anyhow!(
            "o nome do projeto só pode conter letras, números e sublinhados (_), sem espaços"
        ));
    }
    if let Some(first) = name.chars().next() {
        if !first.is_ascii_alphabetic() {
            return Err(anyhow!("o nome do projeto deve começar com uma letra"));
        }
    }

    let parent_path = Path::new(parent_dir);
    if !parent_path.exists() {
        std::fs::create_dir_all(parent_path)
            .with_context(|| format!("não foi possível criar o diretório pai {:?}", parent_path))?;
    }

    let project_dir = parent_path.join(name);
    if project_dir.exists() {
        return Err(anyhow!("a pasta do projeto já existe: {:?}", project_dir));
    }

    std::fs::create_dir_all(&project_dir)
        .with_context(|| format!("não foi possível criar a pasta do projeto {:?}", project_dir))?;

    let engine_assoc = crate::engine::short_version(&engine.version);

    let template_folder_name = match (template_key, is_cpp) {
        ("third_person", true) => "TP_ThirdPerson",
        ("third_person", false) => "TP_ThirdPersonBP",
        ("first_person", true) => "TP_FirstPerson",
        ("first_person", false) => "TP_FirstPersonBP",
        ("top_down", true) => "TP_TopDown",
        ("top_down", false) => "TP_TopDownBP",
        ("vehicle", true) => "TP_VehicleAdv",
        ("vehicle", false) => "TP_VehicleAdvBP",
        (_, true) => "TP_Blank",
        (_, false) => "TP_BlankBP",
    };

    let template_dir = Path::new(&engine.path)
        .join("Templates")
        .join(template_folder_name);

    if template_dir.is_dir() {
        copy_and_customize_template(&template_dir, &project_dir, template_folder_name, name, &engine_assoc)?;
    } else {
        scaffold_clean_project(&project_dir, name, &engine_assoc, is_cpp)?;
    }

    if let Some(rhi) = rhi_target {
        let _ = apply_rhi_to_default_engine_ini(&project_dir, rhi);
    }

    let uproject_path = project_dir.join(format!("{}.uproject", name));

    // Se for C++, tenta gerar os arquivos de workspace/IDE via GenerateProjectFiles.sh se disponível
    if is_cpp {
        let gen_sh = Path::new(&engine.path).join("Engine/Build/BatchFiles/Linux/GenerateProjectFiles.sh");
        if gen_sh.is_file() {
            let _ = Command::new(&gen_sh)
                .arg(format!("-project={}", uproject_path.to_string_lossy()))
                .arg("-game")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }

    let auto_screenshot = project_dir.join("Saved/AutoScreenshot.png");
    let thumbnail_path = auto_screenshot
        .is_file()
        .then(|| auto_screenshot.to_string_lossy().to_string());

    Ok(UnrealProject {
        name: name.to_string(),
        uproject_path: uproject_path.to_string_lossy().to_string(),
        project_dir: project_dir.to_string_lossy().to_string(),
        has_source: is_cpp,
        thumbnail_path,
        engine_association: Some(engine_assoc),
        matched_engine_id: Some(engine.id.clone()),
        rhi_mode: rhi_target.map(|s| s.to_string()),
    })
}

fn apply_rhi_to_default_engine_ini(project_dir: &Path, rhi: &str) -> Result<()> {
    let ini_path = project_dir.join("Config/DefaultEngine.ini");
    let mut content = if ini_path.is_file() {
        std::fs::read_to_string(&ini_path).unwrap_or_default()
    } else {
        String::new()
    };

    let section_header = "[/Script/LinuxTargetPlatform.LinuxTargetSettings]";
    if rhi == "sm5" {
        let sm5_block = r#"
[/Script/LinuxTargetPlatform.LinuxTargetSettings]
-TargetedRHIs=SF_VULKAN_SM6
+TargetedRHIs=SF_VULKAN_SM5
"#;
        if !content.contains(section_header) {
            content.push_str(sm5_block);
        } else {
            content = content.replace("+TargetedRHIs=SF_VULKAN_SM6\n", "");
            content = content.replace("+TargetedRHIs=SF_VULKAN_SM6\r\n", "");
            if !content.contains("TargetedRHIs=SF_VULKAN_SM5") {
                content = content.replace(
                    section_header,
                    &format!("{}\n+TargetedRHIs=SF_VULKAN_SM5", section_header),
                );
            }
        }
    }

    std::fs::write(ini_path, content)?;
    Ok(())
}

fn copy_and_customize_template(
    template_dir: &Path,
    project_dir: &Path,
    template_name: &str,
    new_name: &str,
    engine_assoc: &str,
) -> Result<()> {
    for entry in WalkDir::new(template_dir).into_iter().filter_entry(|e| {
        let fname = e.file_name().to_string_lossy();
        !matches!(
            fname.as_ref(),
            "Media" | "Intermediate" | "Saved" | "Binaries" | "DerivedDataCache" | ".git"
        )
    }) {
        let entry = entry?;
        let rel_path = entry.path().strip_prefix(template_dir)?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }

        let mut dest_path = project_dir.to_path_buf();
        for component in rel_path.components() {
            let comp_str = component.as_os_str().to_string_lossy();
            let new_comp = comp_str.replace(template_name, new_name);
            dest_path.push(new_comp);
        }

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let ext = entry.path().extension().and_then(|s| s.to_str()).unwrap_or("");
            let is_text = matches!(ext, "uproject" | "cs" | "ini" | "cpp" | "h" | "txt");

            if is_text {
                let content = std::fs::read_to_string(entry.path())
                    .unwrap_or_default()
                    .replace(template_name, new_name);

                if ext == "uproject" {
                    if let Ok(mut json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(obj) = json_val.as_object_mut() {
                            obj.insert(
                                "EngineAssociation".to_string(),
                                serde_json::Value::String(engine_assoc.to_string()),
                            );
                        }
                        let pretty = serde_json::to_string_pretty(&json_val)?;
                        std::fs::write(&dest_path, pretty)?;
                        continue;
                    }
                }

                std::fs::write(&dest_path, content)?;
            } else {
                std::fs::copy(entry.path(), &dest_path)?;
            }
        }
    }

    std::fs::create_dir_all(project_dir.join("Content"))?;
    std::fs::create_dir_all(project_dir.join("Config"))?;

    Ok(())
}

fn scaffold_clean_project(
    project_dir: &Path,
    name: &str,
    engine_assoc: &str,
    is_cpp: bool,
) -> Result<()> {
    let config_dir = project_dir.join("Config");
    let content_dir = project_dir.join("Content");
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&content_dir)?;

    let proj_id = uuid::Uuid::new_v4().to_string();

    let def_engine = format!(
r#"[/Script/EngineSettings.GeneralProjectSettings]
ProjectID={}
ProjectName={}
"#,
        proj_id, name
    );
    std::fs::write(config_dir.join("DefaultEngine.ini"), def_engine)?;

    let def_game = format!(
r#"[/Script/EngineSettings.GeneralProjectSettings]
ProjectName={}
"#,
        name
    );
    std::fs::write(config_dir.join("DefaultGame.ini"), def_game)?;

    if is_cpp {
        let source_dir = project_dir.join("Source");
        let module_dir = source_dir.join(name);
        std::fs::create_dir_all(&module_dir)?;

        let target_cs = format!(
r#"using UnrealBuildTool;
using System.Collections.Generic;

public class {name}Target : TargetRules
{{
	public {name}Target(TargetInfo Target) : base(Target)
	{{
		Type = TargetType.Game;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("{name}");
	}}
}}
"#
        );
        std::fs::write(source_dir.join(format!("{}.Target.cs", name)), target_cs)?;

        let editor_target_cs = format!(
r#"using UnrealBuildTool;
using System.Collections.Generic;

public class {name}EditorTarget : TargetRules
{{
	public {name}EditorTarget(TargetInfo Target) : base(Target)
	{{
		Type = TargetType.Editor;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("{name}");
	}}
}}
"#
        );
        std::fs::write(source_dir.join(format!("{}Editor.Target.cs", name)), editor_target_cs)?;

        let build_cs = format!(
r#"using UnrealBuildTool;

public class {name} : ModuleRules
{{
	public {name}(ReadOnlyTargetRules Target) : base(Target)
	{{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

		PublicDependencyModuleNames.AddRange(new string[] {{
			"Core",
			"CoreUObject",
			"Engine",
			"InputCore",
			"EnhancedInput"
		}});

		PrivateDependencyModuleNames.AddRange(new string[] {{ }});
	}}
}}
"#
        );
        std::fs::write(module_dir.join(format!("{}.Build.cs", name)), build_cs)?;

        let header = format!(
r#"#pragma once

#include "CoreMinimal.h"
"#
        );
        std::fs::write(module_dir.join(format!("{}.h", name)), header)?;

        let cpp = format!(
r#"#include "{name}.h"
#include "Modules/ModuleManager.h"

IMPLEMENT_PRIMARY_GAME_MODULE( FDefaultGameModuleImpl, {name}, "{name}" );
"#
        );
        std::fs::write(module_dir.join(format!("{}.cpp", name)), cpp)?;

        let uproject_json = serde_json::json!({
            "FileVersion": 3,
            "EngineAssociation": engine_assoc,
            "Category": "",
            "Description": "",
            "Modules": [
                {
                    "Name": name,
                    "Type": "Runtime",
                    "LoadingPhase": "Default"
                }
            ],
            "Plugins": [
                {
                    "Name": "ModelingToolsEditorMode",
                    "Enabled": true,
                    "TargetAllowList": [
                        "Editor"
                    ]
                }
            ]
        });
        std::fs::write(
            project_dir.join(format!("{}.uproject", name)),
            serde_json::to_string_pretty(&uproject_json)?,
        )?;
    } else {
        let uproject_json = serde_json::json!({
            "FileVersion": 3,
            "EngineAssociation": engine_assoc,
            "Category": "",
            "Description": "",
            "Modules": [],
            "Plugins": [
                {
                    "Name": "ModelingToolsEditorMode",
                    "Enabled": true,
                    "TargetAllowList": [
                        "Editor"
                    ]
                }
            ]
        });
        std::fs::write(
            project_dir.join(format!("{}.uproject", name)),
            serde_json::to_string_pretty(&uproject_json)?,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EngineInstall;

    fn mock_engine() -> EngineInstall {
        EngineInstall {
            id: "test-engine-id".to_string(),
            version: "5.5.3".to_string(),
            path: "/tmp/non-existent-engine".to_string(),
            editor_binary: "/tmp/non-existent-engine/Engine/Binaries/Linux/UnrealEditor".to_string(),
            is_source_build: false,
            label: "Unreal Engine 5.5.3".to_string(),
        }
    }

    #[test]
    fn test_create_blueprint_project_scaffold() {
        let temp_dir = std::env::temp_dir().join(format!("ue_test_bp_{}", uuid::Uuid::new_v4()));
        let engine = mock_engine();

        let proj = create_unreal_project(
            "TestBPGame",
            &temp_dir.to_string_lossy(),
            &engine,
            false,
            "blank",
            Some("sm5"),
        )
        .expect("falha ao criar projeto Blueprint");

        assert_eq!(proj.name, "TestBPGame");
        assert!(!proj.has_source);
        assert_eq!(proj.engine_association, Some("5.5".to_string()));

        let uproject = std::path::Path::new(&proj.uproject_path);
        assert!(uproject.is_file());

        let content = std::fs::read_to_string(uproject).unwrap();
        assert!(content.contains("\"EngineAssociation\": \"5.5\""));

        let config = std::path::Path::new(&proj.project_dir).join("Config/DefaultEngine.ini");
        assert!(config.is_file());
        let config_str = std::fs::read_to_string(config).unwrap();
        assert!(config_str.contains("TargetedRHIs=SF_VULKAN_SM5"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_create_cpp_project_scaffold() {
        let temp_dir = std::env::temp_dir().join(format!("ue_test_cpp_{}", uuid::Uuid::new_v4()));
        let engine = mock_engine();

        let proj = create_unreal_project(
            "TestCppGame",
            &temp_dir.to_string_lossy(),
            &engine,
            true,
            "blank",
            None,
        )
        .expect("falha ao criar projeto C++");

        assert_eq!(proj.name, "TestCppGame");
        assert!(proj.has_source);

        let uproject = std::path::Path::new(&proj.uproject_path);
        assert!(uproject.is_file());

        let source = std::path::Path::new(&proj.project_dir).join("Source/TestCppGame/TestCppGame.Build.cs");
        assert!(source.is_file());

        let target = std::path::Path::new(&proj.project_dir).join("Source/TestCppGame.Target.cs");
        assert!(target.is_file());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_ides() {
        let ides = list_detected_ides();
        assert_eq!(ides.len(), 4);
        assert!(ides.iter().any(|i| i.id == "code"));
        assert!(ides.iter().any(|i| i.id == "rider"));
        assert!(ides.iter().any(|i| i.id == "clion"));
        assert!(ides.iter().any(|i| i.id == "vs"));
    }
}
