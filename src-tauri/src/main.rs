// Evita abrir um console no Windows em builds de release; inofensivo no Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod build;
mod commands;
mod config;
mod credentials;
mod engine;
mod epic;
mod hardware;
mod project;
mod security;
mod updater;
mod vault;

use config::{AppConfig, AppState};
use std::sync::Mutex;

fn main() {
    let config = AppConfig::load();
    let _ = engine::sync_install_ini(&config.engines); // best-effort, não impede o app de abrir

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            config: Mutex::new(config),
        })
        .setup(|app| {
            epic::start_catalog_sync_worker(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_engines,
            commands::add_engine_from_folder,
            commands::remove_engine,
            commands::extract_engine_zip,
            commands::list_project_dirs,
            commands::add_project_dir,
            commands::remove_project_dir,
            commands::scan_projects,
            commands::create_project,
            commands::remove_project,
            commands::delete_project_from_disk,
            commands::read_project_thumbnail,
            commands::launch_project,
            commands::launch_engine_editor,
            commands::get_rhi_settings,
            commands::set_project_rhi_mode,
            commands::set_global_rhi_mode,
            commands::get_gpu_info,
            commands::open_project_in_ide,
            commands::get_available_ides,
            commands::list_vault_items,
            commands::install_vault_to_project,
            commands::install_vault_to_engine,
            commands::create_project_from_vault,
            commands::cancel_vault_download,
            commands::legendary_status,
            commands::epic_status,
            commands::epic_login_url,
            commands::legendary_login_auto,
            commands::epic_login_auto,
            commands::legendary_login,
            commands::epic_login,
            commands::legendary_logout,
            commands::epic_logout,
            commands::epic_list_available_engines,
            commands::epic_get_sso_download_url,
            commands::epic_open_download_window,
            commands::epic_download_and_install,
            commands::epic_pause_download,
            commands::epic_resume_download,
            commands::epic_cancel_download,
            commands::open_path_in_file_manager,
            commands::check_app_update,
            commands::download_and_apply_update,
            commands::get_auto_check_updates,
            commands::set_auto_check_updates,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar a aplicação Tauri");
}
