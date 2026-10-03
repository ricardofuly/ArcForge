//! Validation shared by IPC and download destinations. Never trust a path from the UI.
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn file_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 180 || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        || name == "." || name == ".." {
        bail!("Nome de arquivo inválido");
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()) {
        bail!("Nome reservado pelo Windows");
    }
    Ok(())
}

pub fn project_name(name: &str) -> Result<()> {
    file_name(name)?;
    if !name.starts_with(|c: char| c.is_ascii_alphabetic())
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("O nome do projeto deve começar com uma letra e conter somente letras, números e _");
    }
    Ok(())
}

pub fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 { bail!("Links/junctions não são permitidos neste destino"); }
                }
                if meta.file_type().is_symlink() { bail!("Links não são permitidos neste destino"); }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn monitored_project(path: &Path, dirs: &[String]) -> Result<PathBuf> {
    if !path.is_absolute() { bail!("Selecione um caminho de projeto absoluto"); }
    reject_links(path)?;
    let canonical = path.canonicalize().context("Projeto não encontrado")?;
    if canonical.extension().and_then(|s| s.to_str()) != Some("uproject") || !canonical.is_file() {
        bail!("Selecione um arquivo .uproject válido");
    }
    canonical.parent().ok_or_else(|| anyhow!("Projeto sem pasta"))?;
    if !dirs.iter().filter_map(|d| Path::new(d).canonicalize().ok())
        .any(|root| canonical.starts_with(root)) {
        bail!("Projeto fora das pastas monitoradas");
    }
    let meta = std::fs::metadata(&canonical)?;
    if meta.len() > 2 * 1024 * 1024 { bail!("Descritor de projeto muito grande"); }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&canonical)?)?;
    if !json.is_object() || json.get("FileVersion").and_then(|v| v.as_u64()).is_none() {
        bail!("Descritor .uproject inválido");
    }
    Ok(canonical)
}

pub fn thumbnail(path: &Path, dirs: &[String]) -> Result<Vec<u8>> {
    reject_links(path)?;
    let path = path.canonicalize()?;
    let saved = path.parent().ok_or_else(|| anyhow!("Thumbnail sem pasta"))?;
    if saved.file_name().and_then(|s| s.to_str()) != Some("Saved") { bail!("Thumbnail fora da pasta Saved"); }
    let parent = saved.parent().ok_or_else(|| anyhow!("Thumbnail sem projeto"))?;
    if path.file_name().and_then(|s| s.to_str()) != Some("AutoScreenshot.png")
        || !parent.read_dir()?.filter_map(|e| e.ok()).any(|e| monitored_project(&e.path(), dirs).is_ok()) {
        bail!("Thumbnail fora de um projeto monitorado");
    }
    // Bound the read even if the file grows after metadata was inspected.
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("Thumbnail PNG inválida ou maior que 8 MB");
    }
    Ok(bytes)
}

pub fn engine_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.fragment().is_none() && url.port_or_known_default() == Some(443)
        && matches!(url.host_str(), Some("ucs-blob-store.s3-accelerate.amazonaws.com"
            | "epicgames-engine-builds.s3.amazonaws.com"))
        && (url.path().ends_with(".zip") || url.path().starts_with("/blobs/"))
}

pub fn engine_http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !engine_url(attempt.url()) { attempt.error("Redirecionamento de download não autorizado") }
            else { attempt.follow() }
        })).timeout(std::time::Duration::from_secs(6 * 60 * 60)).build()?)
}

pub fn callback_url(url: &reqwest::Url, port: u16, nonce: &str) -> bool {
    url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port() == Some(port)
        && url.path() == "/download" && url.query_pairs().any(|(k, v)| k == "nonce" && v == nonce)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_devices_and_untrusted_urls() {
        for name in ["../outside.zip", "..\\outside.zip", "C:\\x", "x:stream", "NUL.zip", "CON", "COM1.txt", "x.", "x\n"] { assert!(file_name(name).is_err(), "{name}"); }
        assert!(file_name("Linux_Unreal_Engine_5.8.2.zip").is_ok());
        for url in ["http://epicgames-engine-builds.s3.amazonaws.com/x.zip", "https://evil.invalid/x.zip", "https://192.168.1.10/x.zip", "https://epicgames-engine-builds.s3.amazonaws.com.evil.invalid/x.zip", "https://ucs-blob-store.s3-accelerate.amazonaws.com@evil.invalid/x.zip"] { assert!(!engine_url(&url.parse().unwrap()), "{url}"); }
        assert!(project_name("../overwrite").is_err());
        assert!(project_name("ArcProject_1").is_ok());
    }
    #[test]
    fn callback_requires_nonce_host_path_and_port() {
        let parse = |s: &str| reqwest::Url::parse(s).unwrap();
        assert!(callback_url(&parse("http://127.0.0.1:1234/download?nonce=secret"), 1234, "secret"));
        for url in ["http://127.0.0.1:1234/download", "http://127.0.0.1:1234/download?nonce=wrong", "http://127.0.0.1:1235/download?nonce=secret", "http://evil.invalid:1234/download?nonce=secret"] { assert!(!callback_url(&parse(url), 1234, "secret")); }
    }
    #[test]
    fn scopes_projects_and_checks_thumbnail_contents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let project = root.join("Demo");
        std::fs::create_dir_all(&project).unwrap();
        let dirs = vec![root.to_string_lossy().to_string()];
        let descriptor = project.join("Demo.uproject");
        std::fs::write(&descriptor, r#"{"FileVersion":3}"#).unwrap();
        assert!(monitored_project(&descriptor, &dirs).is_ok());
        assert!(monitored_project(&descriptor, &[]).is_err());
        let arbitrary = project.join("private.txt");
        std::fs::write(&arbitrary, "private").unwrap();
        assert!(monitored_project(&arbitrary, &dirs).is_err());
        assert!(thumbnail(&arbitrary, &dirs).is_err());
        std::fs::create_dir_all(project.join("Saved")).unwrap();
        let png = project.join("Saved/AutoScreenshot.png");
        std::fs::write(&png, b"not a PNG").unwrap();
        assert!(thumbnail(&png, &dirs).is_err());
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        assert!(thumbnail(&png, &dirs).is_ok());
        std::fs::write(project.join("Fake.uproject"), "not JSON").unwrap();
        assert!(monitored_project(&project.join("Fake.uproject"), &dirs).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_windows_junction_destinations() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let junction = tmp.path().join("junction");
        let script = format!("New-Item -ItemType Junction -Path '{}' -Target '{}' | Out-Null", junction.display().to_string().replace('\'', "''"), outside.display().to_string().replace('\'', "''"));
        let status = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status().unwrap();
        assert!(status.success());
        assert!(reject_links(&junction.join("Content/file.txt")).is_err());
        assert!(outside.read_dir().unwrap().next().is_none());
    }
}
