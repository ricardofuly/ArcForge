use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInstall {
    pub id: String,
    pub version: String,
    pub path: String,
    pub editor_binary: String,
    pub is_source_build: bool,
    pub label: String,
}

use std::collections::HashMap;

fn default_rhi_mode() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub engines: Vec<EngineInstall>,
    pub project_dirs: Vec<String>,
    #[serde(default)]
    pub excluded_projects: Vec<String>,
    #[serde(default = "default_rhi_mode")]
    pub rhi_mode: String,
    #[serde(default)]
    pub project_rhi_overrides: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            engines: Vec::new(),
            project_dirs: Vec::new(),
            excluded_projects: Vec::new(),
            rhi_mode: default_rhi_mode(),
            project_rhi_overrides: HashMap::new(),
        }
    }
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
}

fn config_dir() -> PathBuf {
    let base = dirs::config_dir().expect("não foi possível localizar o diretório de config do usuário");
    base.join("unreal-launcher")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => AppConfig::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = config_dir();
        fs::create_dir_all(&dir)?;
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(config_path(), raw)?;
        Ok(())
    }

    pub fn resolve_project_rhi(&self, uproject_path: &str) -> String {
        if let Some(mode) = self.project_rhi_overrides.get(uproject_path) {
            if mode != "auto" {
                return mode.clone();
            }
        }
        if self.rhi_mode != "auto" {
            return self.rhi_mode.clone();
        }
        let gpu = crate::hardware::detect_gpu_info();
        if gpu.recommends_sm5 {
            "sm5".to_string()
        } else {
            "auto".to_string()
        }
    }
}
