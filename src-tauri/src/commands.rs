use crate::config::{AppState, EngineInstall};
use crate::{engine, epic, project};
use tauri::{State, Window};

fn to_err(e: anyhow::Error) -> String {
    e.to_string()
}

fn same_engine_path(a: &str, b: &str) -> bool {
    if cfg!(target_os = "windows") {
        a.trim_end_matches(['/', '\\']).replace('/', "\\").eq_ignore_ascii_case(&b.trim_end_matches(['/', '\\']).replace('/', "\\"))
    } else { a == b }
}

#[tauri::command]
pub async fn epic_status() -> epic::EpicStatus {
    epic::status().await
}

#[tauri::command]
pub async fn legendary_status() -> epic::EpicStatus {
    epic::status().await
}

#[tauri::command]
pub fn epic_login_url() -> &'static str {
    epic::LOGIN_URL
}

#[tauri::command]
pub async fn epic_login_auto(app: tauri::AppHandle, window: Window) -> Result<epic::EpicStatus, String> {
    epic::login_via_embedded_window(&app, &window)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn legendary_login_auto(app: tauri::AppHandle, window: Window) -> Result<epic::EpicStatus, String> {
    epic::login_via_embedded_window(&app, &window)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn epic_login(window: Window, code: String) -> Result<epic::EpicStatus, String> {
    epic::login_with_code(&window, &code)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn legendary_login(window: Window, code: String) -> Result<epic::EpicStatus, String> {
    epic::login_with_code(&window, &code)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn epic_logout() -> Result<(), String> {
    epic::logout().await.map_err(to_err)
}

#[tauri::command]
pub async fn legendary_logout() -> Result<(), String> {
    epic::logout().await.map_err(to_err)
}

#[tauri::command]
pub async fn epic_list_available_engines(force_refresh: Option<bool>) -> Result<Vec<epic::EngineBlob>, String> {
    epic::list_engine_blobs(force_refresh.unwrap_or(false)).await.map_err(to_err)
}

#[tauri::command]
pub async fn epic_get_sso_download_url() -> Result<String, String> {
    epic::get_sso_download_url().await.map_err(to_err)
}

#[tauri::command]
pub async fn epic_open_download_window(
    app: tauri::AppHandle,
    window: Window,
    target_version: Option<String>,
) -> Result<Option<epic::EngineBlob>, String> {
    epic::open_engine_downloader_window(&app, &window, target_version)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn epic_download_and_install(
    window: Window,
    state: State<'_, AppState>,
    blob: epic::EngineBlob,
    dest_dir: String,
) -> Result<EngineInstall, String> {
    let window_for_task = window.clone();
    let engine_root = engine::download_and_extract_engine(&window_for_task, blob, &dest_dir)
        .await
        .map_err(to_err)?;

    let install = engine::detect_engine(&engine_root.to_string_lossy(), false).map_err(to_err)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.excluded_engine_paths.retain(|p| !same_engine_path(p, &install.path));
    cfg.engines.push(install.clone());
    cfg.save().map_err(to_err)?;
    let _ = engine::sync_install_ini(&cfg.engines);
    Ok(install)
}

#[tauri::command]
#[allow(unused_mut)]
pub fn list_engines(state: State<AppState>) -> Vec<EngineInstall> {
    let mut cfg = state.config.lock().unwrap();
    #[cfg(target_os = "windows")]
    {
        let auto = engine::auto_detect_installed_engines();
        let mut changed = false;
        for install in auto {
            if !cfg.engines.iter().any(|e| same_engine_path(&e.path, &install.path))
                && !cfg.excluded_engine_paths.iter().any(|p| same_engine_path(p, &install.path)) {
                cfg.engines.push(install);
                changed = true;
            }
        }
        if changed {
            let _ = cfg.save();
            let _ = engine::sync_install_ini(&cfg.engines);
        }
    }
    cfg.engines.clone()
}

#[tauri::command]
pub async fn add_engine_from_folder(
    state: State<'_, AppState>,
    path: String,
    is_source_build: bool,
) -> Result<EngineInstall, String> {
    let install = tauri::async_runtime::spawn_blocking(move || {
        engine::detect_engine_in(&path, is_source_build)
    })
    .await
    .map_err(|e| format!("falha interna ao procurar a engine: {e}"))?
    .map_err(to_err)?;

    let mut cfg = state.config.lock().unwrap();
    cfg.excluded_engine_paths.retain(|p| !same_engine_path(p, &install.path));
    cfg.engines.push(install.clone());
    cfg.save().map_err(to_err)?;
    let _ = engine::sync_install_ini(&cfg.engines);
    Ok(install)
}

#[tauri::command]
pub fn remove_engine(state: State<AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if let Some(engine) = cfg.engines.iter().find(|e| e.id == id).cloned() {
        if !cfg.excluded_engine_paths.iter().any(|p| same_engine_path(p, &engine.path)) {
            cfg.excluded_engine_paths.push(engine.path);
        }
    }
    cfg.engines.retain(|e| e.id != id);
    cfg.save().map_err(to_err)?;
    let _ = engine::sync_install_ini(&cfg.engines);
    Ok(())
}

#[tauri::command]
pub async fn extract_engine_zip(
    window: Window,
    state: State<'_, AppState>,
    zip_path: String,
    dest_dir: String,
) -> Result<EngineInstall, String> {
    let window_for_thread = window.clone();
    let engine_root = tauri::async_runtime::spawn_blocking(move || {
        engine::extract_engine_zip(&window_for_thread, &zip_path, &dest_dir)
    })
    .await
    .map_err(|e| format!("falha interna ao agendar a extração: {e}"))?
    .map_err(to_err)?;

    let install = engine::detect_engine(&engine_root.to_string_lossy(), false).map_err(to_err)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.excluded_engine_paths.retain(|p| !same_engine_path(p, &install.path));
    cfg.engines.push(install.clone());
    cfg.save().map_err(to_err)?;
    let _ = engine::sync_install_ini(&cfg.engines);
    Ok(install)
}

#[tauri::command]
pub fn list_project_dirs(state: State<AppState>) -> Vec<String> {
    state.config.lock().unwrap().project_dirs.clone()
}

#[tauri::command]
pub fn add_project_dir(state: State<AppState>, path: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if !cfg.project_dirs.contains(&path) {
        cfg.project_dirs.push(path);
    }
    cfg.save().map_err(to_err)
}

#[tauri::command]
pub fn remove_project_dir(state: State<AppState>, path: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.project_dirs.retain(|p| p != &path);
    cfg.save().map_err(to_err)
}

#[tauri::command]
pub async fn scan_projects(state: State<'_, AppState>) -> Result<Vec<project::UnrealProject>, String> {
    let (dirs, engines, excluded, cfg_clone) = {
        let cfg = state.config.lock().unwrap();
        (cfg.project_dirs.clone(), cfg.engines.clone(), cfg.excluded_projects.clone(), cfg.clone())
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut all = Vec::new();
        for dir in &dirs {
            let mut list = project::scan_directory_for_projects(dir, &engines, &excluded);
            for p in &mut list {
                p.rhi_mode = Some(cfg_clone.resolve_project_rhi(&p.uproject_path));
            }
            all.extend(list);
        }
        all
    })
    .await
    .map_err(|e| format!("falha interna ao escanear projetos: {e}"))?;

    Ok(result)
}

#[tauri::command]
pub fn remove_project(state: State<AppState>, uproject_path: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if !cfg.excluded_projects.contains(&uproject_path) {
        cfg.excluded_projects.push(uproject_path);
    }
    cfg.save().map_err(to_err)
}

#[tauri::command]
pub fn delete_project_from_disk(state: State<AppState>, uproject_path: String) -> Result<(), String> {
    let p = std::path::Path::new(&uproject_path);
    if !p.exists() {
        return Err("o arquivo .uproject não foi encontrado no caminho especificado".to_string());
    }

    let project_dir = p.parent().ok_or_else(|| "diretório do projeto inválido".to_string())?;
    let canonical_proj = project_dir.canonicalize().map_err(|e| format!("caminho inválido: {e}"))?;

    // Verificações de segurança estritas para impedir remoção acidental de diretórios críticos
    let root = std::path::Path::new("/");
    if canonical_proj == root || canonical_proj.parent().is_none() {
        return Err("operação abortada por segurança: não é permitido excluir o diretório raiz".to_string());
    }

    // Proibir exclusão de diretórios padrão de usuário (Home, Desktop, Downloads, Documentos, etc.)
    let mut protected_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(h) = dirs::home_dir() { protected_dirs.push(h); }
    if let Some(d) = dirs::desktop_dir() { protected_dirs.push(d); }
    if let Some(d) = dirs::document_dir() { protected_dirs.push(d); }
    if let Some(d) = dirs::download_dir() { protected_dirs.push(d); }
    if let Some(p) = dirs::picture_dir() { protected_dirs.push(p); }
    if let Some(a) = dirs::audio_dir() { protected_dirs.push(a); }
    if let Some(v) = dirs::video_dir() { protected_dirs.push(v); }

    for protected in protected_dirs {
        if let Ok(canon_prot) = protected.canonicalize() {
            if canonical_proj == canon_prot {
                return Err(format!(
                    "operação abortada por segurança: não é permitido excluir diretórios padrão do usuário ('{}')",
                    canon_prot.display()
                ));
            }
        }
    }

    // Garante que a pasta contém realmente o arquivo .uproject esperado antes de apagar
    let uproject_canon = p.canonicalize().map_err(|e| format!("arquivo .uproject inválido: {e}"))?;
    if !uproject_canon.starts_with(&canonical_proj) {
        return Err("operação abortada: o arquivo .uproject não pertence ao diretório informado".to_string());
    }

    // Exclui a pasta do projeto do disco
    std::fs::remove_dir_all(&canonical_proj)
        .map_err(|e| format!("falha ao excluir pasta do projeto: {e}"))?;

    // Registra como excluído para atualizar a lista
    let mut cfg = state.config.lock().unwrap();
    if !cfg.excluded_projects.contains(&uproject_path) {
        cfg.excluded_projects.push(uproject_path);
    }
    let _ = cfg.save();

    Ok(())

}

#[tauri::command]
pub async fn create_project(
    state: State<'_, AppState>,
    name: String,
    parent_dir: String,
    engine_id: String,
    project_type: String,
    template: String,
    rhi_target: Option<String>,
) -> Result<project::UnrealProject, String> {
    let (engine, target_rhi) = {
        let cfg = state.config.lock().unwrap();
        let engine = cfg
            .engines
            .iter()
            .find(|e| e.id == engine_id)
            .cloned()
            .ok_or_else(|| "engine selecionada não encontrada".to_string())?;

        let rhi = rhi_target.unwrap_or_else(|| {
            let gpu = crate::hardware::detect_gpu_info();
            if gpu.recommends_sm5 {
                "sm5".to_string()
            } else {
                "auto".to_string()
            }
        });

        (engine, rhi)
    };

    let is_cpp = project_type.to_lowercase() == "cpp";

    let proj = tauri::async_runtime::spawn_blocking(move || {
        project::create_unreal_project(&name, &parent_dir, &engine, is_cpp, &template, Some(&target_rhi))
    })
    .await
    .map_err(|e| format!("erro na thread de criação: {e}"))?
    .map_err(to_err)?;

    Ok(proj)
}

#[tauri::command]
pub async fn read_project_thumbnail(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("não consegui ler a thumbnail: {e}"))?;
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        Ok(format!("data:image/png;base64,{encoded}"))
    })
    .await
    .map_err(|e| format!("falha interna ao ler a thumbnail: {e}"))?
}

#[tauri::command]
pub fn launch_engine_editor(
    state: State<AppState>,
    engine_id: String,
    rhi_mode: Option<String>,
) -> Result<(), String> {
    let (engine, resolved_rhi) = {
        let cfg = state.config.lock().unwrap();
        let engine = cfg
            .engines
            .iter()
            .find(|e| e.id == engine_id)
            .cloned()
            .ok_or_else(|| "engine não encontrada".to_string())?;

        let rhi = if let Some(m) = rhi_mode {
            if m == "auto" {
                let gpu = crate::hardware::detect_gpu_info();
                if gpu.recommends_sm5 {
                    "sm5".to_string()
                } else {
                    "auto".to_string()
                }
            } else {
                m
            }
        } else {
            let gpu = crate::hardware::detect_gpu_info();
            if gpu.recommends_sm5 {
                "sm5".to_string()
            } else {
                "auto".to_string()
            }
        };

        (engine, rhi)
    };

    engine::launch_editor(&engine, Some(&resolved_rhi)).map_err(to_err)
}

#[tauri::command]
pub fn open_project_in_ide(
    state: State<AppState>,
    project_dir: String,
    ide: String,
) -> Result<(), String> {
    // 1. Sincroniza Install.ini tanto no host quanto nos diretórios de sandboxes Flatpak
    {
        let cfg = state.config.lock().unwrap();
        let _ = crate::engine::sync_install_ini(&cfg.engines);

    }

    let ide = project::Ide::from_str(&ide).map_err(to_err)?;
    project::open_in_ide(&project_dir, ide).map_err(to_err)
}

#[tauri::command]
pub fn get_available_ides() -> Result<Vec<project::IdeInfo>, String> {
    Ok(project::list_detected_ides())
}

#[tauri::command]
pub async fn launch_project(
    window: Window,
    state: State<'_, AppState>,
    uproject_path: String,
    engine_id: String,
    rhi_mode: Option<String>,
) -> Result<(), String> {
    let guard = crate::build::LaunchGuard::acquire().map_err(to_err)?;
    let (engine, resolved_rhi) = {
        let cfg = state.config.lock().unwrap();
        let engine = cfg
            .engines
            .iter()
            .find(|e| e.id == engine_id)
            .cloned()
            .ok_or_else(|| "engine selecionada não encontrada".to_string())?;

        let rhi = if let Some(m) = rhi_mode {
            if m == "auto" {
                cfg.resolve_project_rhi(&uproject_path)
            } else {
                m
            }
        } else {
            cfg.resolve_project_rhi(&uproject_path)
        };

        (engine, rhi)
    };

    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        crate::build::compile(&window, &uproject_path, &engine)?;
        project::launch_project(&uproject_path, &engine, Some(&resolved_rhi))
    })
    .await
    .map_err(|e| format!("Erro interno ao compilar/abrir projeto: {e}"))?
    .map_err(to_err)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RhiSettingsResponse {
    pub global_mode: String,
    pub gpu: crate::hardware::GpuInfo,
    pub project_overrides: std::collections::HashMap<String, String>,
}

#[tauri::command]
pub fn get_rhi_settings(state: State<AppState>) -> Result<RhiSettingsResponse, String> {
    let cfg = state.config.lock().unwrap();
    let gpu = crate::hardware::detect_gpu_info();
    Ok(RhiSettingsResponse {
        global_mode: cfg.rhi_mode.clone(),
        gpu,
        project_overrides: cfg.project_rhi_overrides.clone(),
    })
}

#[tauri::command]
pub fn set_project_rhi_mode(
    state: State<AppState>,
    uproject_path: String,
    rhi_mode: String,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if rhi_mode == "auto" {
        cfg.project_rhi_overrides.remove(&uproject_path);
    } else {
        cfg.project_rhi_overrides.insert(uproject_path.clone(), rhi_mode);
    }
    cfg.save().map_err(to_err)?;

    // Sincroniza o DefaultEngine.ini do projeto
    if let Some(parent) = std::path::Path::new(&uproject_path).parent() {
        let _ = project::ensure_project_target_rhis(parent);
    }

    Ok(())
}

#[tauri::command]
pub fn set_global_rhi_mode(state: State<AppState>, rhi_mode: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.rhi_mode = rhi_mode;
    cfg.save().map_err(to_err)
}

#[tauri::command]
pub fn get_gpu_info() -> Result<crate::hardware::GpuInfo, String> {
    Ok(crate::hardware::detect_gpu_info())
}

#[tauri::command]
pub fn epic_pause_download() {
    crate::engine::DOWNLOAD_PAUSED.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub fn epic_resume_download() {
    crate::engine::DOWNLOAD_PAUSED.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub fn epic_cancel_download() {
    crate::engine::DOWNLOAD_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::engine::DOWNLOAD_PAUSED.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub fn open_path_in_file_manager(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let target = if p.is_file() {
        p.parent().unwrap_or(p)
    } else {
        p
    };
    open::that(target).map_err(|e| format!("falha ao abrir gerenciador de arquivos: {e}"))?;
    Ok(())
}


#[tauri::command]
pub async fn list_vault_items(force_refresh: Option<bool>) -> Result<Vec<crate::vault::VaultItem>, String> {
    crate::vault::list_vault_items(force_refresh.unwrap_or(false))
        .await
        .map_err(to_err)
}

#[tauri::command]
pub async fn install_vault_to_project(
    window: Window,
    catalog_item_id: String,
    app_id: String,
    uproject_path: String,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(uproject_path);
    let target = crate::vault::VaultInstallTarget::Project { uproject_path: path };
    let res = crate::vault::download_and_install_vault_item(&window, &catalog_item_id, &app_id, target)
        .await
        .map_err(to_err)?;
    Ok(res.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn install_vault_to_engine(
    window: Window,
    state: State<'_, AppState>,
    catalog_item_id: String,
    app_id: String,
    engine_id: String,
) -> Result<String, String> {
    let engine_path = {
        let cfg = state.config.lock().unwrap();
        let engine = cfg
            .engines
            .iter()
            .find(|e| e.id == engine_id)
            .cloned()
            .ok_or_else(|| "engine não encontrada".to_string())?;
        std::path::PathBuf::from(engine.path)
    };

    let target = crate::vault::VaultInstallTarget::Engine { engine_path };
    let res = crate::vault::download_and_install_vault_item(&window, &catalog_item_id, &app_id, target)
        .await
        .map_err(to_err)?;
    Ok(res.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn create_project_from_vault(
    window: Window,
    state: State<'_, AppState>,
    catalog_item_id: String,
    app_id: String,
    project_name: String,
    parent_dir: String,
    engine_id: String,
) -> Result<project::UnrealProject, String> {
    let (engine_assoc, engines) = {
        let cfg = state.config.lock().unwrap();
        let assoc = cfg
            .engines
            .iter()
            .find(|e| e.id == engine_id)
            .map(|e| crate::engine::short_version(&e.version))
            .unwrap_or_else(|| engine_id.clone());
        (assoc, cfg.engines.clone())
    };

    let parent_path = std::path::PathBuf::from(&parent_dir);
    let target = crate::vault::VaultInstallTarget::NewProject {
        project_name: project_name.clone(),
        parent_dir: parent_path,
        engine_id: engine_assoc,
    };

    let dest_dir = crate::vault::download_and_install_vault_item(&window, &catalog_item_id, &app_id, target)
        .await
        .map_err(to_err)?;

    // Garante configurações corretas de SM5 e SM6 no projeto baixado do Vault
    let _ = project::ensure_project_target_rhis(&dest_dir);

    // Sincroniza Install.ini no host e nas sandboxes de Flatpaks
    let _ = crate::engine::sync_install_ini(&engines);

    // Escanear projeto após criação
    let (engines, excluded) = {
        let cfg = state.config.lock().unwrap();
        (cfg.engines.clone(), cfg.excluded_projects.clone())
    };

    let projects = project::scan_directory_for_projects(&dest_dir.to_string_lossy(), &engines, &excluded);
    let created = projects.into_iter().next().ok_or_else(|| "projeto criado com sucesso no disco".to_string())?;

    Ok(created)
}

#[tauri::command]
pub fn cancel_vault_download() {
    crate::vault::VAULT_DOWNLOAD_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub async fn check_app_update() -> Result<crate::updater::UpdateInfo, String> {
    crate::updater::check_for_updates().await
}

#[tauri::command]
pub async fn download_and_apply_update(
    app: tauri::AppHandle,
    asset_url: String,
    asset_name: String,
) -> Result<(), String> {
    crate::updater::download_and_apply_update(asset_url, asset_name, app).await
}

#[tauri::command]
pub fn get_auto_check_updates(state: State<'_, AppState>) -> bool {
    let cfg = state.config.lock().unwrap();
    cfg.auto_check_updates
}

#[tauri::command]
pub fn set_auto_check_updates(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.auto_check_updates = enabled;
    cfg.save().map_err(to_err)
}

