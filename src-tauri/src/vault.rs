use anyhow::{anyhow, Context, Result};
use egs_api::api::types::chunk::Chunk;
use egs_api::api::types::download_manifest::DownloadManifest;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Window};

pub static VAULT_DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VaultItemType {
    Plugin,
    Project,
    AssetPack,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultRelease {
    pub id: String,
    pub app_id: String,
    #[serde(default)]
    pub version_title: String,
    #[serde(default)]
    pub compatible_apps: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub date_added: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultItem {
    pub id: String, // catalog_item_id
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub developer: String,
    pub item_type: VaultItemType,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub featured_url: Option<String>,
    #[serde(default)]
    pub releases: Vec<VaultRelease>,
    #[serde(default)]
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultCache {
    pub last_updated: u64,
    pub items: Vec<VaultItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultProgressEvent {
    pub item_id: String,
    pub title: String,
    pub stage: String, // "manifest", "downloading", "extracting", "completed", "error"
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub current_file: String,
    pub percentage: f32,
    pub speed_bytes_per_sec: u64,
    pub error: Option<String>,
}

fn vault_cache_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("não consegui determinar diretório de config"))?
        .join("unreal-launcher");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir.join("vault_cache.json"))
}

pub fn load_cached_vault() -> Option<Vec<VaultItem>> {
    let path = vault_cache_path().ok()?;
    load_cached_vault_from(&path)
}

pub fn load_cached_vault_from(path: &Path) -> Option<Vec<VaultItem>> {
    if !path.exists() {
        return None;
    }
    let data = fs::read_to_string(path).ok()?;
    let cache: VaultCache = serde_json::from_str(&data).ok()?;
    Some(cache.items)
}

pub fn save_cached_vault(items: &[VaultItem]) -> Result<()> {
    let path = vault_cache_path()?;
    save_cached_vault_to(&path, items)
}

pub fn save_cached_vault_to(path: &Path, items: &[VaultItem]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cache = VaultCache {
        last_updated: now,
        items: items.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)?;

    // Gravação atômica (.tmp + rename) para prevenir corrupção em caso de fechamento abrupto
    let tmp_path = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, json)?;
    if fs::rename(&tmp_path, path).is_err() {
        let _ = fs::remove_file(path);
        if fs::rename(&tmp_path, path).is_err() {
            fs::copy(&tmp_path, path)?;
            let _ = fs::remove_file(&tmp_path);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EpicAssetEntry {
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    #[serde(rename = "catalogItemId")]
    pub catalog_item_id: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogKeyImage {
    #[serde(rename = "type")]
    pub img_type: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct CatalogCategory {
    pub path: String,
}

#[derive(Debug, Deserialize)]
struct CatalogReleaseInfo {
    pub id: Option<String>,
    #[serde(rename = "appId")]
    pub app_id: Option<String>,
    #[serde(rename = "versionTitle")]
    pub version_title: Option<String>,
    #[serde(rename = "compatibleApps")]
    pub compatible_apps: Option<Vec<String>>,
    pub platform: Option<Vec<String>>,
    #[serde(rename = "dateAdded")]
    pub date_added: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogItemDetails {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub developer: Option<String>,
    pub categories: Option<Vec<CatalogCategory>>,
    #[serde(rename = "keyImages")]
    pub key_images: Option<Vec<CatalogKeyImage>>,
    #[serde(rename = "releaseInfo")]
    pub release_info: Option<Vec<CatalogReleaseInfo>>,
    pub namespace: Option<String>,
}

/// Consulta o catálogo da Epic Games para os itens do vault Unreal pertencentes ao usuário.
pub async fn fetch_user_vault_items() -> Result<Vec<VaultItem>> {
    let access_token = crate::epic::get_valid_access_token().await?;
    let client = crate::epic::create_client()?;

    // 1. Obter todos os ativos da conta do usuário
    let assets_url = "https://launcher-public-service-prod06.ol.epicgames.com/launcher/api/public/assets/Windows?label=Live";
    let resp = client
        .get(assets_url)
        .bearer_auth(&access_token)
        .send()
        .await
        .context("falha ao consultar ativos da Epic Games")?;

    if !resp.status().is_success() {
        return Err(anyhow!("status de erro da Epic ao listar ativos: {}", resp.status()));
    }

    let entries: Vec<EpicAssetEntry> = resp.json().await.context("erro ao decodificar ativos")?;

    // Filtrar namespace "ue" (Marketplace / Fab / Vault Unreal) e agrupar catalogItemIds únicos
    let mut unique_catalog_ids: Vec<String> = entries
        .into_iter()
        .filter(|e| e.namespace.as_deref() == Some("ue") && e.catalog_item_id.is_some())
        .filter_map(|e| e.catalog_item_id)
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();

    if unique_catalog_ids.is_empty() {
        return Ok(Vec::new());
    }

    unique_catalog_ids.sort();

    // 2. Buscar detalhes em lotes de 50 catalogItemIds usando bulk/items em paralelo
    let chunks: Vec<Vec<String>> = unique_catalog_ids
        .chunks(50)
        .map(|c| c.to_vec())
        .collect();

    let batch_results: Vec<Result<Vec<VaultItem>>> = stream::iter(chunks)
        .map(|batch| {
            let client = client.clone();
            let access_token = access_token.clone();
            async move {
                let query = batch
                    .iter()
                    .map(|id| format!("id={id}"))
                    .collect::<Vec<String>>()
                    .join("&");

                let bulk_url = format!(
                    "https://catalog-public-service-prod06.ol.epicgames.com/catalog/api/shared/namespace/ue/bulk/items?{query}"
                );

                let batch_resp = client
                    .get(&bulk_url)
                    .bearer_auth(&access_token)
                    .send()
                    .await?;

                if !batch_resp.status().is_success() {
                    return Ok(Vec::new());
                }

                let map: HashMap<String, CatalogItemDetails> = batch_resp.json().await?;
                let mut batch_items = Vec::new();

                for (_id, details) in map {
                    let categories = details.categories.unwrap_or_default();

                    // Ignora itens de engines puras instaláveis
                    let is_engine = categories.iter().any(|c| {
                        let path = c.path.to_lowercase();
                        path.starts_with("engines")
                    });
                    if is_engine {
                        continue;
                    }

                    let title = details.title.unwrap_or_else(|| "Sem título".to_string());
                    let description = details.description.unwrap_or_default();
                    let developer = details.developer.unwrap_or_else(|| "Epic Games".to_string());
                    let namespace = details.namespace.unwrap_or_else(|| "ue".to_string());

                    // Determinar tipo do item baseado nas categorias
                    let mut item_type = VaultItemType::Other;
                    for cat in &categories {
                        let p = cat.path.to_lowercase();
                        if p.contains("plugin") {
                            item_type = VaultItemType::Plugin;
                            break;
                        } else if p.contains("project") {
                            item_type = VaultItemType::Project;
                            break;
                        } else if p.contains("asset") || p.contains("prop") || p.contains("environment") || p.contains("material") {
                            item_type = VaultItemType::AssetPack;
                        }
                    }
                    if item_type == VaultItemType::Other && !categories.is_empty() {
                        item_type = VaultItemType::AssetPack;
                    }

                    // Imagens
                    let images = details.key_images.unwrap_or_default();
                    let thumbnail_url = images
                        .iter()
                        .find(|img| img.img_type.eq_ignore_ascii_case("Thumbnail"))
                        .or_else(|| images.iter().find(|img| img.img_type.eq_ignore_ascii_case("Featured")))
                        .or_else(|| images.iter().find(|img| img.img_type.eq_ignore_ascii_case("Screenshot")))
                        .or_else(|| images.first())
                        .map(|img| img.url.clone());

                    let featured_url = images
                        .iter()
                        .find(|img| img.img_type.eq_ignore_ascii_case("Featured"))
                        .or_else(|| images.iter().find(|img| img.img_type.eq_ignore_ascii_case("Screenshot")))
                        .map(|img| img.url.clone());

                    // Releases
                    let mut releases: Vec<VaultRelease> = details
                        .release_info
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|r| {
                            let app_id = r.app_id?;
                            Some(VaultRelease {
                                id: r.id.unwrap_or_else(|| app_id.clone()),
                                app_id,
                                version_title: r.version_title.unwrap_or_default(),
                                compatible_apps: r.compatible_apps.unwrap_or_default(),
                                platforms: r.platform.unwrap_or_default(),
                                date_added: r.date_added,
                            })
                        })
                        .collect();

                    // Ordenar releases para colocar as mais novas primeiro
                    releases.sort_by(|a, b| b.app_id.cmp(&a.app_id));

                    batch_items.push(VaultItem {
                        id: details.id,
                        title,
                        description,
                        developer,
                        item_type,
                        thumbnail_url,
                        featured_url,
                        releases,
                        namespace,
                    });
                }

                Ok(batch_items)
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;

    let mut all_vault_items: Vec<VaultItem> = Vec::new();
    for r in batch_results {
        if let Ok(batch_items) = r {
            all_vault_items.extend(batch_items);
        }
    }

    if !all_vault_items.is_empty() {
        all_vault_items.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        let _ = save_cached_vault(&all_vault_items);
    }

    Ok(all_vault_items)
}

/// Retorna a lista de itens do vault. Se force_refresh for falso e houver cache, responde instantaneamente.
/// Em caso de erro de rede ou de API, recorre ao cache local se este existir.
pub async fn list_vault_items(force_refresh: bool) -> Result<Vec<VaultItem>> {
    if !force_refresh {
        if let Some(cached) = load_cached_vault() {
            if !cached.is_empty() {
                return Ok(cached);
            }
        }
    }

    match fetch_user_vault_items().await {
        Ok(items) => Ok(items),
        Err(e) => {
            if let Some(cached) = load_cached_vault() {
                if !cached.is_empty() {
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

#[derive(Debug, Clone)]
pub enum VaultInstallTarget {
    Project { uproject_path: PathBuf },
    Engine { engine_path: PathBuf },
    NewProject { project_name: String, parent_dir: PathBuf, engine_id: String },
}

/// Baixa o manifesto e todos os chunks da release e os instala no destino configurado.
pub async fn download_and_install_vault_item(
    window: &Window,
    catalog_item_id: &str,
    app_id: &str,
    target: VaultInstallTarget,
) -> Result<PathBuf> {
    VAULT_DOWNLOAD_CANCELLED.store(false, Ordering::SeqCst);

    let access_token = crate::epic::get_valid_access_token().await?;
    let client = crate::epic::create_client()?;

    // 1. Notifica início da obtenção do manifesto
    let _ = window.emit("vault-download-progress", VaultProgressEvent {
        item_id: catalog_item_id.to_string(),
        title: "Obtendo manifesto da Epic Games…".to_string(),
        stage: "manifest".to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
        current_file: String::new(),
        percentage: 0.0,
        speed_bytes_per_sec: 0,
        error: None,
    });

    let manifest_api_url = format!(
        "https://launcher-public-service-prod06.ol.epicgames.com/launcher/api/public/assets/v2/platform/Windows/namespace/ue/catalogItem/{catalog_item_id}/app/{app_id}/label/Live"
    );

    let manifest_resp = client
        .get(&manifest_api_url)
        .bearer_auth(&access_token)
        .send()
        .await
        .context("falha ao consultar manifesto de download")?;

    if !manifest_resp.status().is_success() {
        return Err(anyhow!("a API da Epic retornou status {}", manifest_resp.status()));
    }

    let manifest_data: serde_json::Value = manifest_resp.json().await?;
    let elements = manifest_data["elements"]
        .as_array()
        .ok_or_else(|| anyhow!("resposta de manifesto inválida da Epic"))?;
    let elem = elements.first().ok_or_else(|| anyhow!("nenhum elemento no manifesto"))?;
    let manifests = elem["manifests"]
        .as_array()
        .ok_or_else(|| anyhow!("nenhum manifesto disponível para esta release"))?;
    let m = manifests.first().ok_or_else(|| anyhow!("lista de manifestos vazia"))?;

    let base_uri = m["uri"].as_str().ok_or_else(|| anyhow!("URI do manifesto ausente"))?;
    let query_param = m["queryParams"].as_array().and_then(|q| q.first());
    let (q_name, q_val) = match query_param {
        Some(qp) => (
            qp["name"].as_str().unwrap_or_default(),
            qp["value"].as_str().unwrap_or_default(),
        ),
        None => ("", ""),
    };

    let signed_manifest_url = if !q_name.is_empty() {
        format!("{base_uri}?{q_name}={q_val}")
    } else {
        base_uri.to_string()
    };

    let raw_manifest = client
        .get(&signed_manifest_url)
        .send()
        .await
        .context("falha ao baixar arquivo de manifesto binário")?
        .bytes()
        .await?;

    let mut download_manifest = DownloadManifest::parse(raw_manifest.to_vec())
        .ok_or_else(|| anyhow!("falha ao interpretar o arquivo de manifesto binário"))?;

    // Configurar BaseUrl para montagem correta das URLs dos chunks
    let cdn_base = base_uri.rsplit_once('/').map(|(b, _)| b).unwrap_or(base_uri);
    if let Some(ref mut fields) = download_manifest.custom_fields {
        fields.insert("BaseUrl".to_string(), cdn_base.to_string());
    } else {
        download_manifest.custom_fields = Some(HashMap::from([("BaseUrl".to_string(), cdn_base.to_string())]));
    }

    let files_map = download_manifest.files();
    if files_map.is_empty() {
        return Err(anyhow!("o manifesto não possui arquivos"));
    }

    let total_uncompressed_bytes = download_manifest.total_size() as u64;

    // Criar diretório temporário para receber chunks baixados
    let temp_chunks_dir = std::env::temp_dir()
        .join("unreal_launcher_vault")
        .join(format!("{catalog_item_id}_{app_id}"));
    let _ = fs::remove_dir_all(&temp_chunks_dir);
    fs::create_dir_all(&temp_chunks_dir)?;

    // 2. Mapear todos os chunks únicos necessários
    let mut unique_chunks: HashMap<String, String> = HashMap::new(); // guid -> download_url
    for file in files_map.values() {
        for part in &file.file_chunk_parts {
            if let Some(link) = &part.link {
                let full_url = if !q_name.is_empty() {
                    format!("{link}?{q_name}={q_val}")
                } else {
                    link.to_string()
                };
                unique_chunks.entry(part.guid.clone()).or_insert(full_url);
            }
        }
    }

    let total_chunks = unique_chunks.len();
    let downloaded_chunks_count = Arc::new(AtomicU64::new(0));
    let downloaded_bytes_accum = Arc::new(AtomicU64::new(0));
    let start_time = Instant::now();

    // 3. Download paralelo dos chunks (usando pool de semáforo)
    let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
    let mut download_handles = Vec::new();

    for (guid, chunk_url) in unique_chunks {
        let sem = semaphore.clone();
        let client = client.clone();
        let chunk_file_path = temp_chunks_dir.join(format!("{guid}.raw"));
        let downloaded_count = downloaded_chunks_count.clone();
        let downloaded_bytes = downloaded_bytes_accum.clone();
        let window_clone = window.clone();
        let cat_id = catalog_item_id.to_string();

        let handle = tokio::spawn(async move {
            if VAULT_DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
                return Ok(());
            }

            let _permit = sem.acquire().await.map_err(|e| anyhow!("{e}"))?;

            let resp = client.get(&chunk_url).send().await?;
            let bytes = resp.bytes().await?;
            let chunk_data_len = bytes.len() as u64;

            // Decodificar chunk binário da Epic e descompactar
            let chunk = Chunk::from_vec(bytes.to_vec())
                .ok_or_else(|| anyhow!("chunk corrompido ou inválido: {guid}"))?;

            fs::write(&chunk_file_path, &chunk.data)?;

            let current_count = downloaded_count.fetch_add(1, Ordering::SeqCst) + 1;
            let current_bytes = downloaded_bytes.fetch_add(chunk_data_len, Ordering::SeqCst) + chunk_data_len;

            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.1 {
                (current_bytes as f64 / elapsed) as u64
            } else {
                0
            };
            let pct = if total_chunks > 0 {
                (current_count as f32 / total_chunks as f32) * 85.0
            } else {
                0.0
            };

            let _ = window_clone.emit("vault-download-progress", VaultProgressEvent {
                item_id: cat_id,
                title: format!("Baixando partes: {current_count}/{total_chunks}"),
                stage: "downloading".to_string(),
                downloaded_bytes: current_bytes,
                total_bytes: total_uncompressed_bytes,
                current_file: format!("chunk_{guid}.raw"),
                percentage: pct,
                speed_bytes_per_sec: speed,
                error: None,
            });

            Ok::<(), anyhow::Error>(())
        });

        download_handles.push(handle);
    }

    for h in download_handles {
        if VAULT_DOWNLOAD_CANCELLED.load(Ordering::SeqCst) {
            let _ = fs::remove_dir_all(&temp_chunks_dir);
            return Err(anyhow!("download cancelado pelo usuário"));
        }
        h.await.map_err(|e| anyhow!("tarefa de download abortada: {e}"))??;
    }

    // 4. Determinar pasta de destino e estrutura dos arquivos
    let _ = window.emit("vault-download-progress", VaultProgressEvent {
        item_id: catalog_item_id.to_string(),
        title: "Reconstruindo e instalando arquivos…".to_string(),
        stage: "extracting".to_string(),
        downloaded_bytes: total_uncompressed_bytes,
        total_bytes: total_uncompressed_bytes,
        current_file: String::new(),
        percentage: 90.0,
        speed_bytes_per_sec: 0,
        error: None,
    });

    // Detectar se há arquivo .uproject ou .uplugin para inferir o nome do pacote e caminho base.
    // Se houver .uproject, este pacote é um projeto completo (que pode conter plugins internos em Plugins/).
    let uproject_file = files_map.keys().find(|k| k.to_lowercase().ends_with(".uproject")).cloned();
    let plugin_file = if uproject_file.is_none() {
        files_map.keys().find(|k| k.to_lowercase().ends_with(".uplugin")).cloned()
    } else {
        None
    };

    let target_dest_dir: PathBuf = match &target {
        VaultInstallTarget::Project { uproject_path } => {
            let proj_dir = uproject_path.parent().unwrap_or_else(|| Path::new("."));
            if let Some(ref p_file) = plugin_file {
                let plugin_folder_name = Path::new(p_file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Plugin");
                proj_dir.join("Plugins").join(plugin_folder_name)
            } else {
                // Asset Pack puro: instala em Content
                proj_dir.join("Content")
            }
        }
        VaultInstallTarget::Engine { engine_path } => {
            let plugin_folder_name = plugin_file
                .as_ref()
                .and_then(|p| Path::new(p).file_stem()?.to_str())
                .unwrap_or_else(|| "MarketplacePlugin");
            engine_path.join("Engine").join("Plugins").join("Marketplace").join(plugin_folder_name)
        }
        VaultInstallTarget::NewProject { project_name, parent_dir, engine_id: _ } => {
            parent_dir.join(project_name)
        }
    };

    fs::create_dir_all(&target_dest_dir)?;

    // Identificar prefixo para remoção (ex: "ProjectName/" ou "Engine/Plugins/Marketplace/PluginName/")
    let strip_prefix: Option<String> = if let Some(ref uf) = uproject_file {
        Path::new(uf).parent().and_then(|p| {
            let s = p.to_string_lossy().to_string();
            if s.is_empty() || s == "." {
                None
            } else {
                Some(s)
            }
        })
    } else if let Some(ref pf) = plugin_file {
        Path::new(pf).parent().and_then(|p| {
            let s = p.to_string_lossy().to_string();
            if s.is_empty() || s == "." {
                None
            } else {
                Some(s)
            }
        })
    } else {
        None
    };

    // 5. Montar cada arquivo gravando os pedaços descompactados com sanitização de segurança e buffer de I/O
    for (filename, file_manifest) in files_map {
        let relative_path = if let Some(ref prefix) = strip_prefix {
            if filename.starts_with(prefix) {
                let rest = &filename[prefix.len()..];
                rest.trim_start_matches('/').to_string()
            } else {
                filename.clone()
            }
        } else {
            filename.clone()
        };

        // Prevenção estrita contra Path Traversal / Zip Slip
        let safe_rel = match sanitize_relative_path(&relative_path) {
            Some(p) => p,
            None => {
                eprintln!("[VAULT SEGURANÇA] Caminho perigoso ou inválido ignorado: {}", filename);
                continue;
            }
        };

        let file_dest = target_dest_dir.join(safe_rel);
        if let Some(parent) = file_dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let out_file = fs::File::create(&file_dest)?;
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, out_file);
        for part in &file_manifest.file_chunk_parts {
            let chunk_raw_path = temp_chunks_dir.join(format!("{}.raw", part.guid));
            if chunk_raw_path.exists() {
                let chunk_data = fs::read(&chunk_raw_path)?;
                let offset = part.offset as usize;
                let size = part.size as usize;
                if offset + size <= chunk_data.len() {
                    writer.write_all(&chunk_data[offset..offset + size])?;
                }
            }
        }
        writer.flush()?;
    }


    // Se o alvo for criação de novo projeto e havia um uproject, renomeia para o novo nome
    if let VaultInstallTarget::NewProject { project_name, parent_dir: _, engine_id } = &target {
        // Localiza qualquer .uproject extraído na raiz do novo projeto
        if let Ok(entries) = fs::read_dir(&target_dest_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("uproject") {
                    let target_uproject = target_dest_dir.join(format!("{project_name}.uproject"));
                    if p != target_uproject {
                        let _ = fs::rename(&p, &target_uproject);
                    }
                    // Atualizar EngineAssociation
                    if let Ok(content) = fs::read_to_string(&target_uproject) {
                        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                            json["EngineAssociation"] = serde_json::Value::String(engine_id.clone());
                            let _ = fs::write(&target_uproject, serde_json::to_string_pretty(&json)?);
                        }
                    }
                    break;
                }
            }
        }
    }

    // Limpeza da pasta de chunks temporários
    let _ = fs::remove_dir_all(&temp_chunks_dir);

    // Conclusão com sucesso
    let _ = window.emit("vault-download-progress", VaultProgressEvent {
        item_id: catalog_item_id.to_string(),
        title: "Instalação concluída com sucesso!".to_string(),
        stage: "completed".to_string(),
        downloaded_bytes: total_uncompressed_bytes,
        total_bytes: total_uncompressed_bytes,
        current_file: String::new(),
        percentage: 100.0,
        speed_bytes_per_sec: 0,
        error: None,
    });

    Ok(target_dest_dir)
}

/// Sanitiza um caminho relativo oriundo do manifesto para impedir vulnerabilidades de Path Traversal (Zip Slip).
/// Retorna None caso o caminho tente retroceder (..), acessar a raiz (/ ou drive C:), ou contenha caracteres proibidos.
pub fn sanitize_relative_path(path: &str) -> Option<PathBuf> {
    let normalized = path.replace('\\', "/");
    let mut clean = PathBuf::new();

    for component in Path::new(&normalized).components() {
        match component {
            std::path::Component::Normal(c) => {
                let s = c.to_string_lossy();
                // Proibir caracteres de controle e especificadores de drive (ex: C:)
                if s.contains(':') || s.contains('\0') {
                    return None;
                }
                clean.push(c);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                // Tentativa de escapar da raiz ou subir diretórios rejeitada
                return None;
            }
        }
    }

    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_relative_path_safety() {
        // Caminhos válidos
        assert_eq!(
            sanitize_relative_path("Source/MyPlugin/MyPlugin.Build.cs"),
            Some(PathBuf::from("Source/MyPlugin/MyPlugin.Build.cs"))
        );
        assert_eq!(
            sanitize_relative_path("Content\\Textures\\Icon.png"),
            Some(PathBuf::from("Content/Textures/Icon.png"))
        );

        // Tentativas de Path Traversal (Zip Slip)
        assert_eq!(sanitize_relative_path("../../../etc/passwd"), None);
        assert_eq!(sanitize_relative_path("foo/../../bar"), None);
        assert_eq!(sanitize_relative_path("/root/secrets.txt"), None);
        assert_eq!(sanitize_relative_path("C:\\Windows\\System32\\cmd.exe"), None);
        assert_eq!(sanitize_relative_path("C:autoexec.bat"), None);
        assert_eq!(sanitize_relative_path(".."), None);
        assert_eq!(sanitize_relative_path(""), None);
        assert_eq!(sanitize_relative_path("./"), None);
    }

    #[test]
    fn test_vault_cache_save_and_load() {
        let temp_dir = std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4()));
        let test_cache_path = temp_dir.join("vault_cache.json");

        let item = VaultItem {
            id: "test_item_123".to_string(),
            title: "Test Plugin".to_string(),
            description: "A wonderful test plugin".to_string(),
            developer: "Antigravity".to_string(),
            item_type: VaultItemType::Plugin,
            thumbnail_url: Some("https://example.com/thumb.png".to_string()),
            featured_url: None,
            releases: vec![VaultRelease {
                id: "rel_1".to_string(),
                app_id: "TestPlugin_5.5".to_string(),
                version_title: "Version 1.0".to_string(),
                compatible_apps: vec!["UE_5.5".to_string()],
                platforms: vec!["Linux".to_string()],
                date_added: None,
            }],
            namespace: "ue".to_string(),
        };

        assert!(save_cached_vault_to(&test_cache_path, &[item.clone()]).is_ok());
        let loaded = load_cached_vault_from(&test_cache_path).expect("deve carregar cache");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "Test Plugin");
        assert_eq!(loaded[0].releases.len(), 1);
        assert_eq!(loaded[0].releases[0].app_id, "TestPlugin_5.5");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_fetch_user_vault_items() {
        let res = fetch_user_vault_items().await;
        println!("Live fetch result: {:?}", res.as_ref().map(|v| v.len()));
        assert!(res.is_ok());
    }
}
