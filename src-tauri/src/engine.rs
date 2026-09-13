use crate::config::EngineInstall;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{Emitter, Window};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;
use walkdir::WalkDir;

pub static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);
pub static DOWNLOAD_PAUSED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize)]
struct BuildVersion {
    #[serde(rename = "MajorVersion")]
    major: u32,
    #[serde(rename = "MinorVersion")]
    minor: u32,
    #[serde(rename = "PatchVersion")]
    patch: u32,
}

#[cfg(target_os = "windows")]
const EDITOR_RELATIVE_PATH: &str = "Engine/Binaries/Win64/UnrealEditor.exe";
#[cfg(not(target_os = "windows"))]
const EDITOR_RELATIVE_PATH: &str = "Engine/Binaries/Linux/UnrealEditor";

fn editor_binary_path(engine_root: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let ue5 = engine_root.join("Engine/Binaries/Win64/UnrealEditor.exe");
        if ue5.is_file() {
            return ue5;
        }
        let ue4 = engine_root.join("Engine/Binaries/Win64/UE4Editor.exe");
        if ue4.is_file() {
            return ue4;
        }
        ue5
    }
    #[cfg(not(target_os = "windows"))]
    {
        let ue5 = engine_root.join("Engine/Binaries/Linux/UnrealEditor");
        if ue5.is_file() {
            return ue5;
        }
        let ue4 = engine_root.join("Engine/Binaries/Linux/UE4Editor");
        if ue4.is_file() {
            return ue4;
        }
        ue5
    }
}

fn read_build_version(engine_root: &Path) -> Result<String> {
    let build_version_path = engine_root.join("Engine/Build/Build.version");
    let raw = std::fs::read_to_string(&build_version_path)
        .with_context(|| format!("não encontrei Engine/Build/Build.version em {:?}", engine_root))?;
    let parsed: BuildVersion = serde_json::from_str(&raw)
        .with_context(|| "Build.version não pôde ser interpretado como JSON válido")?;
    Ok(format!("{}.{}.{}", parsed.major, parsed.minor, parsed.patch))
}

/// Monta o registro de uma engine já validada (root é exatamente a pasta que contém 'Engine/').
fn build_install(root: &Path, is_source_build: bool) -> Result<EngineInstall> {
    let binary = editor_binary_path(root);
    if !binary.is_file() {
        return Err(anyhow!(
            "não encontrei o binário do editor em {:?}",
            binary
        ));
    }

    let version = read_build_version(root).unwrap_or_else(|_| "desconhecida".to_string());

    Ok(EngineInstall {
        id: Uuid::new_v4().to_string(),
        label: format!("Unreal Engine {}", version),
        version,
        path: root.to_string_lossy().to_string(),
        editor_binary: binary.to_string_lossy().to_string(),
        is_source_build,
    })
}

/// Valida uma pasta como instalação de engine (precisa ser exatamente a raiz) e monta o registro.
/// Usado no fluxo de extração de zip, onde já sabemos a raiz exata.
pub fn detect_engine(engine_root: &str, is_source_build: bool) -> Result<EngineInstall> {
    let root = PathBuf::from(engine_root);
    if !root.is_dir() {
        return Err(anyhow!("o caminho informado não é uma pasta válida"));
    }
    build_install(&root, is_source_build)
}

/// Procura pela raiz de uma instalação da engine dentro de `search_dir`, olhando também
/// nas subpastas — assim o usuário pode apontar pra uma pasta "guarda-chuva" (ex: ~/UnrealEngines)
/// sem precisar navegar manualmente até a pasta exata que contém 'Engine/'.
fn find_engine_root(search_dir: &Path) -> Result<PathBuf> {
    // Caminho mais comum: a pasta escolhida já é a raiz da engine.
    if editor_binary_path(search_dir).is_file() {
        return Ok(search_dir.to_path_buf());
    }

    // Senão, varre as subpastas. Pula diretórios pesados que nunca levam ao binário do
    // editor Linux, pra não gastar tempo à toa dentro de uma instalação de ~30-40GB.
    let walker = WalkDir::new(search_dir).max_depth(8).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            "Content" | "Intermediate" | "Saved" | "DerivedDataCache" | ".git"
                | "Source" | "Extras" | "Documentation" | "Samples" | "Templates"
                | "FeaturePacks" | "Config"
        )
    });

    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file()
            && (path.ends_with(EDITOR_RELATIVE_PATH)
                || path.ends_with("Engine/Binaries/Win64/UE4Editor.exe")
                || path.ends_with("Engine/Binaries/Linux/UE4Editor"))
        {
            // path é .../<raiz>/Engine/Binaries/<Plataforma>/UnrealEditor — sobe 4 níveis até a raiz.
            if let Some(root) = path
                .parent() // Linux / Win64
                .and_then(|p| p.parent()) // Binaries
                .and_then(|p| p.parent()) // Engine
                .and_then(|p| p.parent()) // raiz
            {
                return Ok(root.to_path_buf());
            }
        }
    }

    Err(anyhow!(
        "não encontrei nenhuma instalação da Unreal Engine dentro de {:?} (procurei por Engine/Binaries/Linux/UnrealEditor nas subpastas)",
        search_dir
    ))
}

/// Reduz uma versão tipo "5.8.1" pra "5.8" (major.minor, sem patch) — é o formato usado
/// no campo EngineAssociation dos .uproject e nas chaves do Install.ini.
pub fn short_version(version: &str) -> String {
    version.splitn(3, '.').take(2).collect::<Vec<_>>().join(".")
}
/// Ponto de entrada do fluxo "já está extraída": aceita tanto a raiz exata da engine quanto
/// uma pasta pai, procurando recursivamente pelo binário do editor.
pub fn detect_engine_in(search_dir: &str, is_source_build: bool) -> Result<EngineInstall> {
    let root = PathBuf::from(search_dir);
    if !root.is_dir() {
        return Err(anyhow!("o caminho informado não é uma pasta válida"));
    }
    let engine_root = find_engine_root(&root)?;
    build_install(&engine_root, is_source_build)
}

fn all_install_ini_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 1. Caminho nativo padrão do host (~/.config/Epic/UnrealEngine/Install.ini)
    if let Some(base) = dirs::config_dir() {
        paths.push(base.join("Epic/UnrealEngine/Install.ini"));
    }

    // 2. Caminhos isolados de sandbox de Flatpaks (~/.var/app/*/config/Epic/UnrealEngine/Install.ini)
    // Rider, CLion, VS Code e VSCodium rodam em sandboxes Flatpak com XDG_CONFIG_HOME isolado.
    if let Some(home) = dirs::home_dir() {
        let var_app = home.join(".var/app");
        if var_app.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&var_app) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        paths.push(p.join("config/Epic/UnrealEngine/Install.ini"));
                    }
                }
            }
        }
    }

    paths
}

/// Reescreve o `Install.ini` que o ecossistema Epic usa no Linux (Rider, extensões de VS Code,
/// UnrealBuildTool) pra resolver o campo `EngineAssociation` de um `.uproject` (ex: "5.8") pro
/// caminho real da engine no disco. Sincroniza tanto no host nativo quanto nas pastas de config
/// dos Flatpaks (Rider, VS Code, etc.).
pub fn sync_install_ini(engines: &[EngineInstall]) -> Result<()> {
    let mut body = String::from("[Installations]\n");
    for engine in engines {
        let is_versioned = engine.version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
        if is_versioned {
            let short_ver = short_version(&engine.version);
            if !short_ver.is_empty() {
                body.push_str(&format!("UE_{}={}\n", short_ver, engine.path));
                body.push_str(&format!("{}={}\n", short_ver, engine.path));
            }
            if !engine.version.is_empty() {
                body.push_str(&format!("UE_{}={}\n", engine.version, engine.path));
                body.push_str(&format!("{}={}\n", engine.version, engine.path));
            }
        }
        if !engine.id.is_empty() {
            body.push_str(&format!("{}={}\n", engine.id, engine.path));
            body.push_str(&format!("{{{}}}={}\n", engine.id, engine.path));
        }
    }

    for path in all_install_ini_paths() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &body);
    }

    #[cfg(target_os = "windows")]
    {
        for engine in engines {
            let is_versioned = engine.version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
            if is_versioned {
                let short_ver = short_version(&engine.version);
                if !short_ver.is_empty() {
                    let _ = Command::new("reg")
                        .args(&["add", "HKCU\\Software\\Epic Games\\Unreal Engine\\Builds", "/v", &short_ver, "/t", "REG_SZ", "/d", &engine.path, "/f"])
                        .output();
                }
            }
            if !engine.id.is_empty() {
                let _ = Command::new("reg")
                    .args(&["add", "HKCU\\Software\\Epic Games\\Unreal Engine\\Builds", "/v", &engine.id, "/t", "REG_SZ", "/d", &engine.path, "/f"])
                    .output();
            }
        }
    }

    Ok(())
}

/// No Windows, procura automaticamente instalações oficiais da Epic Games
/// em ProgramData (LauncherInstalled.dat) e Program Files (C:\Program Files\Epic Games\UE_*).
#[allow(dead_code, unused_mut)]
pub fn auto_detect_installed_engines() -> Vec<EngineInstall> {
    let mut detected = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let program_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        let dat_path = PathBuf::from(program_data).join("Epic/UnrealEngineLauncher/LauncherInstalled.dat");
        if dat_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&dat_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(list) = json.get("InstallationList").and_then(|v| v.as_array()) {
                        for item in list {
                            if let Some(loc) = item.get("InstallLocation").and_then(|v| v.as_str()) {
                                if let Ok(install) = detect_engine_in(loc, false) {
                                    if !detected.iter().any(|d: &EngineInstall| d.path == install.path) {
                                        detected.push(install);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        for drive in ["C", "D", "E"] {
            let epic_dir = PathBuf::from(format!("{drive}:\\Program Files\\Epic Games"));
            if epic_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&epic_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            if name.starts_with("UE_") || name.starts_with("Unreal") {
                                if let Ok(install) = detect_engine_in(&path.to_string_lossy(), false) {
                                    if !detected.iter().any(|d: &EngineInstall| d.path == install.path) {
                                        detected.push(install);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    detected
}

/// Conta quantas entradas existem dentro do zip, usado para calcular progresso de extração.
#[cfg(not(target_os = "windows"))]
fn count_zip_entries(zip_path: &str) -> Result<usize> {
    let output = Command::new("unzip").arg("-l").arg(zip_path).output()
        .with_context(|| "falha ao executar 'unzip -l' — verifique se o pacote 'unzip' está instalado")?;
    if !output.status.success() {
        return Err(anyhow!("'unzip -l' falhou ao ler o arquivo zip"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A saída do unzip -l tem uma linha de cabeçalho, N linhas de entradas, e um rodapé com o total.
    let count = stdout
        .lines()
        .filter(|l| l.trim_start().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .count();
    Ok(count.max(1))
}

/// Extrai um zip de engine pré-compilada emitindo eventos de progresso para a janela.
pub fn extract_engine_zip(window: &Window, zip_path: &str, dest_dir: &str) -> Result<PathBuf> {
    let dest = PathBuf::from(dest_dir);
    std::fs::create_dir_all(&dest)?;

    #[cfg(target_os = "windows")]
    {
        let _ = window.emit("engine-extract-progress", serde_json::json!({ "percent": 10.0, "currentFile": "Iniciando extração no Windows…" }));
        // Windows 10 (build 17063+) e Windows 11 incluem tar.exe nativo que extrai .zip diretamente
        let tar_status = Command::new("tar")
            .arg("-xf")
            .arg(zip_path)
            .arg("-C")
            .arg(&dest)
            .status();

        let success = match tar_status {
            Ok(s) => s.success(),
            Err(_) => {
                let ps_script = format!(
                    "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                    zip_path.replace('\'', "''"),
                    dest.to_string_lossy().replace('\'', "''")
                );
                Command::new("powershell")
                    .args(&["-NoProfile", "-NonInteractive", "-Command", &ps_script])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
        };

        if !success {
            return Err(anyhow!("a extração do arquivo zip falhou no Windows"));
        }

        let _ = window.emit("engine-extract-progress", serde_json::json!({ "percent": 100.0, "currentFile": "concluído" }));

        if let Ok(entries) = std::fs::read_dir(&dest) {
            let subdirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            if subdirs.len() == 1 && subdirs[0].join("Engine").is_dir() {
                return Ok(subdirs[0].clone());
            }
        }
        return Ok(dest);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let total_entries = count_zip_entries(zip_path).unwrap_or(1);

        let mut child = Command::new("unzip")
            .arg("-o") // sobrescreve sem perguntar
            .arg(zip_path)
            .arg("-d")
            .arg(&dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| "falha ao iniciar 'unzip' — verifique se está instalado (sudo apt install unzip)")?;

        let stdout = child.stdout.take().expect("stdout deveria estar disponível");
        let reader = BufReader::new(stdout);

        let mut extracted: usize = 0;
        for line in reader.lines().flatten() {
            if line.contains("inflating:") || line.contains("extracting:") || line.contains("creating:") {
                extracted += 1;
                let percent = ((extracted as f64 / total_entries as f64) * 100.0).min(99.0);
                let _ = window.emit(
                    "engine-extract-progress",
                    serde_json::json!({ "percent": percent, "currentFile": line.trim() }),
                );
            }
        }

        let status = child.wait().with_context(|| "erro aguardando o processo 'unzip'")?;
        if !status.success() {
            return Err(anyhow!("a extração falhou (unzip retornou código de erro)"));
        }

        let _ = window.emit("engine-extract-progress", serde_json::json!({ "percent": 100.0, "currentFile": "concluído" }));

        if let Ok(entries) = std::fs::read_dir(&dest) {
            let subdirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            if subdirs.len() == 1 && subdirs[0].join("Engine").is_dir() {
                return Ok(subdirs[0].clone());
            }
        }

        Ok(dest)
    }
}

/// Abre o editor de uma engine sem nenhum projeto — o Project Browser nativo da Unreal
/// assume a partir daí (criar projeto novo, abrir projeto existente, etc).
pub fn launch_editor(
    engine: &crate::config::EngineInstall,
    rhi_mode: Option<&str>,
) -> Result<()> {
    let binary = PathBuf::from(&engine.editor_binary);
    if !binary.is_file() {
        return Err(anyhow!(
            "binário do editor não encontrado para a engine {}: {:?}",
            engine.label,
            binary
        ));
    }

    let mut cmd = Command::new(&binary);

    // No Linux, força o Unreal Editor a abrir com OpenGL para evitar crash ao fechar janelas dock no Vulkan
    #[cfg(target_os = "linux")]
    {
        cmd.arg("-opengl");
    }

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

/// Faz o download da engine via streaming da URL pré-assinada da S3 com relatório de progresso
/// em tempo real, seguido de extração automática e limpeza do arquivo compactado.
pub async fn download_and_extract_engine(
    window: &Window,
    blob: crate::epic::EngineBlob,
    dest_dir: &str,
) -> Result<PathBuf> {
    use futures_util::StreamExt;
    use std::io::Write;
    use std::time::Instant;

    let dest = PathBuf::from(dest_dir);
    std::fs::create_dir_all(&dest)?;

    let temp_zip_path = dest.join(format!("{}.download", &blob.name));
    let final_zip_path = dest.join(&blob.name);

    if !crate::epic::is_valid_engine_download_url(&blob.url) {
        return Err(anyhow!(
            "a URL obtida para o download não é um link válido para o arquivo .zip da engine: '{}'",
            &blob.url
        ));
    }

    println!("[DOWNLOAD ENGINE] Iniciando transferência: {} -> {}", &blob.name, &blob.url);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(60 * 60 * 6)) // até 6h de timeout para conexões lentas
        .build()?;

    let response = client
        .get(&blob.url)
        .send()
        .await
        .context(format!("falha ao conectar na URL de download da Unreal Engine ({})", &blob.url))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "o servidor da Epic/AWS retornou erro HTTP {} ({}) ao acessar '{}'",
            response.status(),
            response.status().canonical_reason().unwrap_or("erro"),
            &blob.url
        ));
    }

    let total_size = response
        .content_length()
        .unwrap_or(blob.size)
        .max(1);

    let mut file = std::fs::File::create(&temp_zip_path)
        .with_context(|| format!("não consegui criar o arquivo de destino em {:?}", temp_zip_path))?;

    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();
    let mut last_bytes = 0u64;

    let _ = window.emit(
        "engine-download-progress",
        serde_json::json!({
            "percent": 0.0,
            "bytesDownloaded": 0,
            "totalBytes": total_size,
            "speedMbps": 0.0,
        }),
    );

    DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);
    DOWNLOAD_PAUSED.store(false, Ordering::SeqCst);

    let mut stream = response.bytes_stream();

    while let Some(item) = stream.next().await {
        while DOWNLOAD_PAUSED.load(Ordering::SeqCst) {
            if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
                break;
            }
            let _ = window.emit(
                "engine-download-progress",
                serde_json::json!({
                    "percent": ((downloaded as f64 / total_size as f64) * 100.0).min(99.9),
                    "bytesDownloaded": downloaded,
                    "totalBytes": total_size,
                    "speedMbps": 0.0,
                    "isPaused": true,
                }),
            );
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        if DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&temp_zip_path);
            return Err(anyhow!("download cancelado pelo usuário"));
        }

        let chunk = item.context("erro durante a transferência de dados")?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;

        let elapsed = last_emit.elapsed();
        if elapsed.as_millis() >= 300 {
            let bytes_since = downloaded - last_bytes;
            let speed_mbps = (bytes_since as f64 / elapsed.as_secs_f64()) / 1_048_576.0;
            let percent = ((downloaded as f64 / total_size as f64) * 100.0).min(99.9);

            let _ = window.emit(
                "engine-download-progress",
                serde_json::json!({
                    "percent": percent,
                    "bytesDownloaded": downloaded,
                    "totalBytes": total_size,
                    "speedMbps": (speed_mbps * 10.0).round() / 10.0,
                }),
            );

            last_emit = Instant::now();
            last_bytes = downloaded;
        }
    }

    file.flush()?;
    drop(file);

    let _ = window.emit(
        "engine-download-progress",
        serde_json::json!({
            "percent": 100.0,
            "bytesDownloaded": total_size,
            "totalBytes": total_size,
            "speedMbps": 0.0,
        }),
    );

    // Renomeia o arquivo temporário para o zip final
    std::fs::rename(&temp_zip_path, &final_zip_path)
        .context("falha ao renomear arquivo temporário de download")?;

    // Executa a extração usando a função existente com unzip do sistema
    let window_clone = window.clone();
    let zip_str = final_zip_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string();

    let extracted_root = tauri::async_runtime::spawn_blocking(move || {
        extract_engine_zip(&window_clone, &zip_str, &dest_str)
    })
    .await
    .map_err(|e| anyhow!("falha na thread de extração: {e}"))??;

    // Remove o zip baixado para liberar espaço no disco
    let _ = std::fs::remove_file(&final_zip_path);

    Ok(extracted_root)
}

