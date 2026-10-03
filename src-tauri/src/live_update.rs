//! Windows updater runs in a separate process so the mapped executable can be replaced.
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}, process::Command, time::{Duration, Instant}};
use windows_sys::Win32::{Foundation::{CloseHandle, WAIT_OBJECT_0}, System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE}, UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK}};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    parent: u32,
    target: PathBuf,
    version: String,
    asset: String,
}

pub fn prepare(package: &Path, version: &str, asset: &str) -> Result<(), String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?.canonicalize().map_err(|e| e.to_string())?;
    let directory = target.parent().ok_or("Diretório do aplicativo inválido")?;
    // Fail before closing the application. Never elevate a helper from a writable temporary folder.
    tempfile::NamedTempFile::new_in(directory).map_err(|_| "Sem permissão para atualizar esta instalação. Instale o ArcForge para o usuário atual, fora de Program Files, para usar o Live Update.")?;
    let root = package.parent().ok_or("Pacote inválido")?;
    let helper = root.join("arcforge-update-helper.exe");
    fs::copy(&target, &helper).map_err(|e| format!("Falha ao preparar atualizador: {e}"))?;
    let plan = Plan { parent: std::process::id(), target, version: version.into(), asset: asset.into() };
    fs::write(root.join("plan.json"), serde_json::to_vec(&plan).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut child = Command::new(helper).arg("--arcforge-apply-update").spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if root.join("ready").exists() { return Ok(()); }
        if let Some(_) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(fs::read_to_string(root.join("error.log")).unwrap_or_else(|_| "O atualizador não iniciou".into()));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("O atualizador não respondeu; o aplicativo continua aberto".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn run_helper_if_requested() -> bool {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new("--arcforge-apply-update")) { return false; }
    let result = (|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let root = exe.parent().ok_or("Diretório inválido")?;
        let result = apply(root);
        if let Err(error) = &result {
            let log = root.join("error.log");
            let _ = fs::write(&log, error);
            let message: Vec<u16> = format!("Não foi possível concluir a atualização.\n{error}\n\nLog: {}", log.display()).encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "ArcForge — Atualização".encode_utf16().chain(Some(0)).collect();
            if root.join("ready").exists() {
                unsafe { MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR); }
            }
        }
        result
    })();
    if result.is_err() { std::process::exit(1); }
    true
}

fn apply(root: &Path) -> Result<(), String> {
    let plan: Plan = serde_json::from_slice(&fs::read(root.join("plan.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    crate::security::file_name(&plan.asset).map_err(|e| e.to_string())?;
    if !crate::updater::is_live_windows_asset(&plan.asset)
        || !crate::updater::is_newer_version(env!("CARGO_PKG_VERSION"), &plan.version)
        || !plan.target.is_absolute() || plan.target.extension().and_then(|s| s.to_str()) != Some("exe") {
        return Err("Plano de atualização inválido".into());
    }
    let key = option_env!("ARCFORGE_UPDATE_PUBLIC_KEY").ok_or("Chave de assinatura ausente")?;
    let package = root.join(&plan.asset);
    let signature = fs::read_to_string(root.join("package.minisig")).map_err(|e| e.to_string())?;
    // Open and stage the package once; verify the staged bytes, not a path that could change later.
    let mut candidate = tempfile::NamedTempFile::new_in(plan.target.parent().ok_or("Destino inválido")?).map_err(|e| e.to_string())?;
    std::io::copy(&mut fs::File::open(package).map_err(|e| e.to_string())?, &mut candidate).map_err(|e| e.to_string())?;
    candidate.as_file().sync_all().map_err(|e| e.to_string())?;
    crate::updater::verify_update(candidate.path(), key, &signature, &plan.version, &plan.asset)?;
    let candidate = candidate.into_temp_path(); // Close the writable handle before CreateProcess.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, plan.parent) };
    if process.is_null() { return Err("Não foi possível aguardar o aplicativo atual".into()); }
    let ready = fs::write(root.join("ready"), b"ready");
    if let Err(error) = ready { unsafe { CloseHandle(process); } return Err(error.to_string()); }
    let _ = fs::write(root.join("update.log"), "Aguardando encerramento do aplicativo");
    let waited = unsafe { WaitForSingleObject(process, 60_000) };
    unsafe { CloseHandle(process); }
    if waited != WAIT_OBJECT_0 { return Err("O aplicativo atual não encerrou; nenhum arquivo foi substituído".into()); }
    let _ = fs::write(root.join("update.log"), "Aplicativo encerrado. Substituindo executável");
    let result = replace_and_restart(&plan.target, &candidate, |path| {
        Command::new(path).current_dir(path.parent().unwrap()).spawn().map(|_| ()).map_err(|e| e.to_string())
    });
    if result.is_err() { let _ = Command::new(&plan.target).spawn(); }
    if result.is_ok() {
        // Leave the running helper itself for normal OS temporary-directory cleanup.
        for name in [&plan.asset, "package.minisig", "plan.json", "ready"] { let _ = fs::remove_file(root.join(name)); }
    }
    result
}

fn replace_and_restart(target: &Path, candidate: &Path, restart: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let backup = target.with_file_name(format!("arcforge-{}.old", uuid::Uuid::new_v4()));
    fs::rename(target, &backup).map_err(|e| format!("Falha ao preservar a versão anterior: {e}"))?;
    if let Err(error) = fs::rename(candidate, target) {
        fs::rename(&backup, target).map_err(|e| format!("Falha ao restaurar: {e}; backup: {}", backup.display()))?;
        return Err(format!("Falha ao aplicar atualização: {error}"));
    }
    if let Err(error) = restart(target) {
        fs::remove_file(target).map_err(|e| format!("Falha ao retirar atualização: {e}; backup: {}", backup.display()))?;
        fs::rename(&backup, target).map_err(|e| format!("Falha ao restaurar: {e}; backup: {}", backup.display()))?;
        return Err(format!("Falha ao reiniciar; versão anterior restaurada: {error}"));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_preserves_original_when_candidate_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe"); fs::write(&target, b"old").unwrap();
        assert!(replace_and_restart(&target, &dir.path().join("missing"), |_| panic!("must not restart")).is_err());
        assert_eq!(fs::read(target).unwrap(), b"old");
    }
    #[test]
    fn restart_failure_restores_original() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe"); let candidate = dir.path().join("new");
        fs::write(&target, b"old").unwrap(); fs::write(&candidate, b"new").unwrap();
        assert!(replace_and_restart(&target, &candidate, |_| Err("launch failed".into())).is_err());
        assert_eq!(fs::read(target).unwrap(), b"old");
    }
    #[test]
    fn successful_update_restarts_from_original_path_and_cleans_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe"); let candidate = dir.path().join("new");
        fs::write(&target, b"old").unwrap(); fs::write(&candidate, b"new").unwrap();
        replace_and_restart(&target, &candidate, |path| { assert_eq!(path, target); assert_eq!(fs::read(path).unwrap(), b"new"); Ok(()) }).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn staged_executable_can_actually_start_after_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe");
        fs::write(&target, b"previous version").unwrap();
        let mut staged = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        std::io::copy(&mut fs::File::open(std::env::current_exe().unwrap()).unwrap(), &mut staged).unwrap();
        staged.as_file().sync_all().unwrap();
        let candidate = staged.into_temp_path();
        replace_and_restart(&target, &candidate, |path| {
            let output = Command::new(path).arg("--list").output().map_err(|e| e.to_string())?;
            if !output.status.success() { return Err("New executable failed to start".into()); }
            Ok(())
        }).unwrap();
    }
}
