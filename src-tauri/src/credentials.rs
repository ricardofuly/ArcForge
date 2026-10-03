//! Secure session persistence. Failure never writes a plaintext fallback.
use anyhow::{anyhow, Result};
use std::path::Path;

pub fn save(entry: &keyring::Entry, legacy: &Path, session: &str) -> Result<()> {
    entry.set_password(session).map_err(|_| anyhow!("Não foi possível salvar a sessão no cofre do sistema"))?;
    remove_legacy(legacy)
}

fn remove_legacy(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(anyhow!("Não foi possível remover o arquivo de sessão antigo")),
    }
}

pub fn load(entry: &keyring::Entry, legacy: &Path) -> Result<Option<String>> {
    match entry.get_password() {
        Ok(data) => { remove_legacy(legacy)?; Ok(Some(data)) },
        Err(keyring::Error::NoEntry) => {
            crate::security::reject_links(legacy)?;
            let data = match std::fs::read_to_string(legacy) {
                Ok(data) => data,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(_) => return Err(anyhow!("Não foi possível migrar a sessão antiga")),
            };
            // Only migrate a valid Epic session, and delete plaintext after durable storage.
            let session: crate::epic::EpicSession = serde_json::from_str(&data)?;
            let clean = serde_json::to_string(&session)?;
            save(entry, legacy, &clean)?;
            Ok(Some(clean))
        }
        Err(_) => Err(anyhow!("Cofre de credenciais indisponível; nenhum fallback em texto puro será usado")),
    }
}

pub fn clear(entry: &keyring::Entry, legacy: &Path) -> Result<()> {
    // Remove any legacy copy even if the secure store is temporarily unavailable.
    remove_legacy(legacy)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(anyhow!("Não foi possível remover a sessão do cofre do sistema")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> keyring::Entry {
        keyring::Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()))
    }
    const SESSION: &str = r#"{"access_token":"dummy-access","refresh_token":"dummy-refresh","account_id":"test","display_name":"Test","expires_at":0}"#;
    #[cfg(windows)]
    #[test]
    #[ignore = "Writes and deletes an isolated dummy credential in Windows Credential Manager"]
    fn native_windows_credential_store_roundtrip() {
        let service = format!("arcforge-security-test-{}", uuid::Uuid::new_v4());
        let entry = keyring::Entry::new(&service, "dummy").unwrap();
        struct Cleanup<'a>(&'a keyring::Entry);
        impl Drop for Cleanup<'_> { fn drop(&mut self) { let _ = self.0.delete_credential(); } }
        let _cleanup = Cleanup(&entry);
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("session.json");
        std::fs::write(&legacy, SESSION).unwrap();
        assert!(load(&entry, &legacy).unwrap().is_some());
        assert!(!legacy.exists());
        assert!(entry.get_password().unwrap().contains("dummy-access"));
        clear(&entry, &legacy).unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
    #[test]
    fn migrates_once_and_logout_removes_both_stores() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("session.json");
        let entry = entry();
        std::fs::write(&legacy, SESSION).unwrap();
        assert!(load(&entry, &legacy).unwrap().is_some());
        assert!(!legacy.exists());
        assert!(entry.get_password().unwrap().contains("dummy-refresh"));
        std::fs::write(&legacy, SESSION).unwrap();
        assert!(load(&entry, &legacy).unwrap().is_some());
        assert!(!legacy.exists());
        clear(&entry, &legacy).unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
        assert!(load(&entry, &legacy).unwrap().is_none());
    }
    #[test]
    fn failed_secure_write_never_creates_plaintext_and_invalid_legacy_is_not_migrated() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("session.json");
        let entry = entry();
        let mock: &keyring::mock::MockCredential = entry.get_credential().downcast_ref().unwrap();
        mock.set_error(keyring::Error::Invalid("store".into(), "unavailable".into()));
        assert!(save(&entry, &legacy, SESSION).is_err());
        assert!(!legacy.exists());
        std::fs::write(&legacy, SESSION).unwrap();
        mock.set_error(keyring::Error::NoEntry);
        // Invalid JSON is never copied into secure storage.
        std::fs::write(&legacy, "not a session").unwrap();
        assert!(load(&entry, &legacy).is_err());
        assert!(legacy.exists());
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
