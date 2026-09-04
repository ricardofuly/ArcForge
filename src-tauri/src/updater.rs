use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;

const GITHUB_REPO: &str = "ricardofuly/UnrealLauncher-Linux";
const USER_AGENT: &str = "Unreal-Launcher-AutoUpdater";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_name: String,
    pub release_notes: String,
    pub published_at: String,
    pub html_url: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub asset_size: u64,
    pub is_appimage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProgressPayload {
    pub status: String, // "downloading", "installing", "ready", "error"
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percentage: f32,
    pub speed_mbps: f32,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// Compara duas versões em formato SemVer (ex: "0.1.0" e "v0.1.1").
/// Retorna `true` se `remote` for estritamente mais recente que `local`.
pub fn is_newer_version(local: &str, remote: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|part| {
                // Remove qualquer sufixo como -beta, -rc1
                let num_str = part.split('-').next().unwrap_or(part);
                num_str.parse::<u64>().ok()
            })
            .collect()
    };

    let l = parse(local);
    let r = parse(remote);

    // Compara elemento por elemento
    let max_len = l.len().max(r.len());
    for i in 0..max_len {
        let l_val = l.get(i).copied().unwrap_or(0);
        let r_val = r.get(i).copied().unwrap_or(0);
        if r_val > l_val {
            return true;
        } else if r_val < l_val {
            return false;
        }
    }
    false
}

/// Seleciona o asset apropriado da release com base no sistema operacional
fn select_platform_asset(assets: &[GithubAsset]) -> Option<&GithubAsset> {
    #[cfg(target_os = "windows")]
    {
        // No Windows, prioriza instaladores .exe (setup/nsis) ou .msi
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with("-setup.exe") || a.name.ends_with(".exe")) {
            return Some(asset);
        }
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".msi")) {
            return Some(asset);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let is_appimage = std::env::var("APPIMAGE").is_ok();
        if is_appimage {
            if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".AppImage")) {
                return Some(asset);
            }
        }

        // Se não for AppImage, checa o formato do sistema
        if Path::new("/etc/debian_version").exists() {
            if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".deb")) {
                return Some(asset);
            }
        }
        if Path::new("/etc/fedora-release").exists() || Path::new("/etc/redhat-release").exists() {
            if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".rpm")) {
                return Some(asset);
            }
        }

        // Fallback padrão universal para Linux: AppImage
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".AppImage")) {
            return Some(asset);
        }
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".deb")) {
            return Some(asset);
        }
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".rpm")) {
            return Some(asset);
        }
    }

    // Fallback genérico caso não encontre pelas extensões acima
    assets.first()
}

/// Tenta encontrar um token do GitHub (variáveis de ambiente ou configuração do GitHub CLI)
fn find_github_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("UNREAL_LAUNCHER_GITHUB_TOKEN")) {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }

    // Tenta ler o token do GitHub CLI (~/.config/gh/hosts.yml)
    if let Some(config_dir) = dirs::config_dir() {
        let gh_hosts = config_dir.join("gh").join("hosts.yml");
        if let Ok(content) = std::fs::read_to_string(gh_hosts) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("oauth_token:") {
                    if let Some(token) = trimmed.strip_prefix("oauth_token:") {
                        let token = token.trim();
                        if !token.is_empty() {
                            return Some(token.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Consulta a API do GitHub para obter a última release disponível
pub async fn check_for_updates() -> Result<UpdateInfo, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("Erro ao inicializar cliente HTTP: {}", e))?;

    let mut req = client.get(&url).header("Accept", "application/vnd.github.v3+json");

    if let Some(token) = find_github_token() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Falha ao conectar com o GitHub: {}", e))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(
            "Nenhuma release pública encontrada (GitHub 404). \
            O repositório ainda não possui releases publicadas ou o perfil no GitHub está temporariamente restrito para visitantes anônimos. \
            Para que o Live Update funcione para todos os usuários sem login, verifique a visibilidade da conta no GitHub."
                .to_string(),
        );
    }

    if !resp.status().is_success() {
        return Err(format!(
            "GitHub API retornou status {} ao verificar atualizações",
            resp.status()
        ));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| format!("Erro ao decodificar dados da release: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let has_update = is_newer_version(&current_version, &release.tag_name);

    let is_appimage = std::env::var("APPIMAGE").is_ok();
    let chosen_asset = select_platform_asset(&release.assets);

    Ok(UpdateInfo {
        has_update,
        current_version,
        latest_version,
        release_name: release.name.unwrap_or(release.tag_name),
        release_notes: release.body.unwrap_or_default(),
        published_at: release.published_at.unwrap_or_default(),
        html_url: release.html_url,
        asset_name: chosen_asset.map(|a| a.name.clone()),
        asset_url: chosen_asset.map(|a| a.browser_download_url.clone()),
        asset_size: chosen_asset.map(|a| a.size).unwrap_or(0),
        is_appimage,
    })
}

/// Faz o download do asset da atualização com streaming de progresso e dispara a aplicação
pub async fn download_and_apply_update(
    asset_url: String,
    asset_name: String,
    app: AppHandle,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("Erro ao inicializar cliente HTTP: {}", e))?;

    let mut req = client.get(&asset_url);
    if let Some(token) = find_github_token() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let res = req
        .send()
        .await
        .map_err(|e| format!("Falha ao conectar para download: {}", e))?;

    if !res.status().is_success() {
        return Err(format!(
            "Erro de download: servidor retornou status {}",
            res.status()
        ));
    }

    let total_bytes = res.content_length().unwrap_or(0);
    let temp_dir = std::env::temp_dir();
    let temp_file_path = temp_dir.join(format!("unreal_launcher_update_{}", asset_name));

    // Abre o arquivo de destino para escrita assíncrona
    let mut file = tokio::fs::File::create(&temp_file_path)
        .await
        .map_err(|e| format!("Erro ao criar arquivo temporário: {}", e))?;

    let mut stream = res.bytes_stream();
    let mut downloaded_bytes: u64 = 0;
    let mut last_emit = Instant::now();
    let mut last_downloaded = 0u64;

    // Emite status inicial
    let _ = app.emit(
        "app_update_progress",
        UpdateProgressPayload {
            status: "downloading".to_string(),
            downloaded_bytes: 0,
            total_bytes,
            percentage: 0.0,
            speed_mbps: 0.0,
            message: "Iniciando download...".to_string(),
        },
    );

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Erro no streaming do download: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Erro ao gravar dados no disco: {}", e))?;

        downloaded_bytes += chunk.len() as u64;

        if last_emit.elapsed().as_millis() >= 250 {
            let elapsed_sec = last_emit.elapsed().as_secs_f32();
            let bytes_diff = downloaded_bytes.saturating_sub(last_downloaded);
            let speed_mbps = if elapsed_sec > 0.0 {
                (bytes_diff as f32 / (1024.0 * 1024.0)) / elapsed_sec
            } else {
                0.0
            };

            let percentage = if total_bytes > 0 {
                (downloaded_bytes as f32 / total_bytes as f32) * 100.0
            } else {
                0.0
            };

            let _ = app.emit(
                "app_update_progress",
                UpdateProgressPayload {
                    status: "downloading".to_string(),
                    downloaded_bytes,
                    total_bytes,
                    percentage,
                    speed_mbps,
                    message: format!(
                        "Baixando: {:.1} MB / {:.1} MB",
                        downloaded_bytes as f32 / (1024.0 * 1024.0),
                        total_bytes as f32 / (1024.0 * 1024.0)
                    ),
                },
            );

            last_emit = Instant::now();
            last_downloaded = downloaded_bytes;
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Erro ao finalizar gravação do arquivo: {}", e))?;
    drop(file);

    // Emite progresso de 100%
    let _ = app.emit(
        "app_update_progress",
        UpdateProgressPayload {
            status: "installing".to_string(),
            downloaded_bytes,
            total_bytes: downloaded_bytes,
            percentage: 100.0,
            speed_mbps: 0.0,
            message: "Download concluído. Aplicando atualização...".to_string(),
        },
    );

    // Aplicação da atualização conforme a plataforma
    apply_update_file(&temp_file_path, &asset_name, app)
}

/// Aplica o arquivo de atualização dependendo do formato e do sistema operacional
fn apply_update_file(update_path: &Path, asset_name: &str, app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if asset_name.ends_with(".exe") {
            // Executa o instalador .exe
            std::process::Command::new(update_path)
                .spawn()
                .map_err(|e| format!("Erro ao iniciar instalador .exe: {}", e))?;

            // Fecha a aplicação para liberar os arquivos para o instalador
            app.exit(0);
            return Ok(());
        } else if asset_name.ends_with(".msi") {
            // Executa o instalador .msi
            std::process::Command::new("msiexec")
                .args(["/i", update_path.to_str().unwrap_or_default()])
                .spawn()
                .map_err(|e| format!("Erro ao iniciar instalador .msi: {}", e))?;

            app.exit(0);
            return Ok(());
        }
    }

    #[cfg(target_os = "linux")]
    {
        // 1. Caso AppImage
        if let Ok(current_appimage_path) = std::env::var("APPIMAGE") {
            let current_path = PathBuf::from(&current_appimage_path);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(update_path, std::fs::Permissions::from_mode(0o755));
            }

            // Faz backup do arquivo antigo antes de sobrescrever
            let backup_path = current_path.with_extension("AppImage.old");
            let _ = std::fs::remove_file(&backup_path);
            let _ = std::fs::rename(&current_path, &backup_path);

            // Copia o novo AppImage para o caminho do executável atual
            if let Err(e) = std::fs::copy(update_path, &current_path) {
                // Tenta restaurar o backup se falhar
                let _ = std::fs::rename(&backup_path, &current_path);
                return Err(format!("Falha ao substituir o AppImage em execução: {}", e));
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&current_path, std::fs::Permissions::from_mode(0o755));
            }

            // Remove o arquivo temporário
            let _ = std::fs::remove_file(update_path);

            // Relauncha a nova versão do AppImage e encerra a atual
            std::process::Command::new(&current_path)
                .spawn()
                .map_err(|e| format!("Erro ao reiniciar nova versão do AppImage: {}", e))?;

            app.exit(0);
            return Ok(());
        }

        // 2. Caso pacote .deb
        if asset_name.ends_with(".deb") {
            // Tenta abrir com o instalador de pacotes do sistema
            if let Err(_) = std::process::Command::new("pkexec")
                .args(["dpkg", "-i", update_path.to_str().unwrap_or_default()])
                .spawn()
            {
                // Fallback: abre o arquivo com o gerenciador padrão do sistema (GNOME Software, etc)
                let _ = open::that(update_path);
            }
            return Ok(());
        }

        // 3. Caso pacote .rpm
        if asset_name.ends_with(".rpm") {
            if let Err(_) = std::process::Command::new("pkexec")
                .args(["rpm", "-Uvh", update_path.to_str().unwrap_or_default()])
                .spawn()
            {
                let _ = open::that(update_path);
            }
            return Ok(());
        }

        // Se for um AppImage baixado sem o app atual ser um AppImage
        if asset_name.ends_with(".AppImage") {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(update_path, std::fs::Permissions::from_mode(0o755));
            }
            let _ = open::that(update_path);
            return Ok(());
        }
    }

    // Fallback genérico: abre a pasta contendo o arquivo baixado
    let _ = open::that(update_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("0.1.0", "0.1.1"));
        assert!(is_newer_version("0.1.0", "v0.1.1"));
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.1.0", "1.0.0"));
        assert!(is_newer_version("0.1.0", "v0.1.0-1"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "v0.1.0"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
        assert!(!is_newer_version("1.0.0", "0.9.9"));
    }
}

