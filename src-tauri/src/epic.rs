use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Window};

pub const LOGIN_URL: &str = "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode";

// Client ID e Secret públicos do Launcher oficial da Epic (launcherAppClient2)
pub const EPIC_CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";
pub const EPIC_CLIENT_SECRET: &str = "daafbccc737745039dffe53d94fc76cf";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) EpicGamesLauncher/17.0.1 UnrealEngine/4.23.0 Chrome/84.0.4147.38 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpicSession {
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
    pub display_name: String,
    pub expires_at: u64, // timestamp Unix em segundos
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpicStatus {
    pub logged_in: bool,
    pub username: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBlob {
    pub name: String,
    pub size: u64,
    pub created_at: String,
    pub url: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub is_precompiled: bool,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ExchangeCodeResponse {
    pub code: String,
}

fn session_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("não consegui determinar o diretório de config"))?
        .join("unreal-launcher");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir.join("epic_session.json"))
}

pub fn load_session() -> Option<EpicSession> {
    let path = session_path().ok()?;
    if !path.exists() {
        return None;
    }
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_session(session: &EpicSession) -> Result<()> {
    let path = session_path()?;
    let json = serde_json::to_string_pretty(session)?;
    fs::write(&path, json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(&path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            let _ = fs::set_permissions(&path, perms);
        }
    }

    Ok(())
}


pub fn clear_session() -> Result<()> {
    if let Ok(path) = session_path() {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

pub fn create_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .cookie_store(true)
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .context("falha ao inicializar o cliente HTTP")
}

/// Obtém um token de acesso válido, renovando automaticamente caso esteja próximo da expiração.
pub async fn get_valid_access_token() -> Result<String> {
    let mut session = load_session().ok_or_else(|| anyhow!("não há sessão da Epic conectada"))?;
    let now = now_secs();

    // Se o token expirar nos próximos 120 segundos, fazemos o refresh
    if now + 120 >= session.expires_at {
        let client = create_client()?;
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", &session.refresh_token),
            ("token_type", "eg1"),
        ];

        let resp = client
            .post("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token")
            .basic_auth(EPIC_CLIENT_ID, Some(EPIC_CLIENT_SECRET))
            .form(&params)
            .send()
            .await
            .context("falha na requisição de renovação de token")?;

        if !resp.status().is_success() {
            let _ = clear_session();
            return Err(anyhow!("sessão expirada — faça login novamente"));
        }

        let token_data: OAuthTokenResponse = resp.json().await.context("resposta de token inválida")?;
        session.access_token = token_data.access_token.clone();
        session.refresh_token = token_data.refresh_token;
        if let Some(name) = token_data.display_name {
            session.display_name = name;
        }
        session.expires_at = now + token_data.expires_in;
        let _ = save_session(&session);
    }

    Ok(session.access_token)
}

/// Verifica o status da conta Epic persistida (tenta renovar token se necessário).
pub async fn status() -> EpicStatus {
    match get_valid_access_token().await {
        Ok(_) => {
            if let Some(session) = load_session() {
                EpicStatus {
                    logged_in: true,
                    username: Some(session.display_name),
                    account_id: Some(session.account_id),
                }
            } else {
                EpicStatus {
                    logged_in: false,
                    username: None,
                    account_id: None,
                }
            }
        }
        Err(_) => EpicStatus {
            logged_in: false,
            username: None,
            account_id: None,
        },
    }
}

/// Extrai o authorizationCode de uma string (JSON completo, HTML com JSON, ou código puro).
fn extract_auth_code(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("código de autorização vazio"));
    }

    // 1. Tenta parsear como JSON direto
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(code) = value.get("authorizationCode").and_then(|v| v.as_str()) {
            return Ok(code.trim().to_string());
        }
    }

    // 2. Procura substring de "authorizationCode" caso o texto contenha tags HTML (ex: <pre>)
    if let Some(idx) = trimmed.find("authorizationCode") {
        let rest = &trimmed[idx + "authorizationCode".len()..];
        if let Some(start_quote) = rest.find('"') {
            let after_quote = &rest[start_quote + 1..];
            if let Some(end_quote) = after_quote.find('"') {
                let code = after_quote[..end_quote].trim();
                if !code.is_empty() {
                    return Ok(code.to_string());
                }
            }
        }
    }

    // 3. Se for código alfanumérico direto
    let clean: String = trimmed.chars().filter(|c| c.is_alphanumeric()).collect();
    if clean.len() >= 20 {
        return Ok(clean);
    }

    Ok(trimmed.to_string())
}

/// Realiza a troca do authorizationCode por tokens OAuth nativamente via HTTP.
pub async fn login_with_code(window: &Window, raw_input: &str) -> Result<EpicStatus> {
    let code = extract_auth_code(raw_input)?;
    let _ = window.emit("epic-log", serde_json::json!({ "line": "Trocando código de autorização por tokens..." }));

    let client = create_client()?;
    let params = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("token_type", "eg1"),
    ];

    let resp = client
        .post("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token")
        .basic_auth(EPIC_CLIENT_ID, Some(EPIC_CLIENT_SECRET))
        .form(&params)
        .send()
        .await
        .context("erro na conexão com o serviço de autenticação da Epic")?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "login falhou — o código de autorização expirou ou já foi usado. Erro: {err_body}"
        ));
    }

    let token_data: OAuthTokenResponse = resp.json().await.context("formato de resposta inesperado da Epic")?;
    let username = token_data.display_name.clone().unwrap_or_else(|| "Usuário Epic".to_string());

    let session = EpicSession {
        access_token: token_data.access_token,
        refresh_token: token_data.refresh_token,
        account_id: token_data.account_id.clone(),
        display_name: username.clone(),
        expires_at: now_secs() + token_data.expires_in,
    };

    save_session(&session)?;
    let _ = window.emit(
        "epic-log",
        serde_json::json!({ "line": format!("Login concluído com sucesso como: {}", username) }),
    );

    Ok(EpicStatus {
        logged_in: true,
        username: Some(username),
        account_id: Some(token_data.account_id),
    })
}

pub async fn logout() -> Result<()> {
    if let Some(session) = load_session() {
        if let Ok(client) = create_client() {
            let url = format!(
                "https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/sessions/kill/{}",
                session.access_token
            );
            let _ = client.delete(&url).send().await;
        }
    }
    clear_session()?;
    Ok(())
}

/// Abre a página de login da Epic numa janela embutida no launcher e captura o authorizationCode automaticamente.
pub async fn login_via_embedded_window(app: &AppHandle, window: &Window) -> Result<EpicStatus> {
    let closed = Arc::new(AtomicBool::new(false));
    let closed_flag = closed.clone();

    // Inicia um servidor TCP temporário em porta aleatória para receber o código via redirecionamento interno
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("falha ao iniciar listener local para captura de autenticação")?;
    listener.set_nonblocking(true)
        .context("falha ao configurar listener local como não-bloqueante")?;
    let port = listener.local_addr()?.port();

    let captured = Arc::new(Mutex::new(None::<String>));
    let captured_for_nav = captured.clone();
    let captured_for_tcp = captured.clone();

    // Gera um nonce criptográfico para garantir que apenas esta janela do webview possa entregar o código
    let session_nonce = uuid::Uuid::new_v4().to_string();
    let nonce_for_tcp = session_nonce.clone();
    let nonce_for_nav = session_nonce.clone();

    // Thread para processar conexão HTTP local caso a navegação chegue ao socket TCP
    let closed_for_tcp = closed.clone();
    std::thread::spawn(move || {
        while !closed_for_tcp.load(Ordering::SeqCst) {
            if captured_for_tcp.lock().unwrap().is_some() {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 8192];
                    if let Ok(n) = stream.read(&mut buf) {
                        let req = String::from_utf8_lossy(&buf[..n]);
                        if let Some(first_line) = req.lines().next() {
                            if let Some(path) = first_line.split_whitespace().nth(1) {
                                if let Ok(parsed) = reqwest::Url::parse(&format!("http://127.0.0.1:{port}{path}")) {
                                    // Valida nonce de segurança
                                    let has_valid_nonce = parsed.query_pairs().any(|(k, v)| k == "nonce" && v == nonce_for_tcp);
                                    if has_valid_nonce {
                                        for (k, v) in parsed.query_pairs() {
                                            if (k == "data" || k == "code" || k == "authorizationCode") && !v.is_empty() {
                                                let mut lock = captured_for_tcp.lock().unwrap();
                                                if lock.is_none() {
                                                    *lock = Some(v.to_string());
                                                }
                                            }
                                        }
                                        let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!DOCTYPE html><html><body><h3 style='font-family:sans-serif;text-align:center;margin-top:20%'>Login concluído com sucesso!</h3></body></html>";
                                        let _ = stream.write_all(resp.as_bytes());
                                        let _ = stream.flush();
                                    } else {
                                        let resp = "HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n";
                                        let _ = stream.write_all(resp.as_bytes());
                                        let _ = stream.flush();
                                    }
                                }
                            }
                        }
                    }
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    });

    let init_script = format!(r#"
        (function() {{
            function scanForCode() {{
                try {{
                    var text = '';
                    if (document.body) {{
                        text = document.body.innerText || document.body.textContent || '';
                    }}
                    if (text && text.indexOf('authorizationCode') !== -1) {{
                        window.location.replace('http://127.0.0.1:{port}/auth?nonce={session_nonce}&data=' + encodeURIComponent(text));
                        return true;
                    }}
                    var pres = document.getElementsByTagName('pre');
                    for (var i = 0; i < pres.length; i++) {{
                        var pText = pres[i].innerText || pres[i].textContent || '';
                        if (pText && pText.indexOf('authorizationCode') !== -1) {{
                            window.location.replace('http://127.0.0.1:{port}/auth?nonce={session_nonce}&data=' + encodeURIComponent(pText));
                            return true;
                        }}
                    }}
                }} catch(e) {{}}
                return false;
            }}

            if (!scanForCode()) {{
                var timer = setInterval(function() {{
                    if (scanForCode()) clearInterval(timer);
                }}, 150);
                document.addEventListener('DOMContentLoaded', scanForCode);
                window.addEventListener('load', scanForCode);
            }}
        }})();
    "#);

    let eval_script = format!(r#"
        (function() {{
            try {{
                var text = '';
                if (document.body) {{
                    text = document.body.innerText || document.body.textContent || '';
                }}
                if (text && text.indexOf('authorizationCode') !== -1) {{
                    window.location.replace('http://127.0.0.1:{port}/auth?nonce={session_nonce}&data=' + encodeURIComponent(text));
                    return;
                }}
                var pres = document.getElementsByTagName('pre');
                for (var i = 0; i < pres.length; i++) {{
                    var pText = pres[i].innerText || pres[i].textContent || '';
                    if (pText && pText.indexOf('authorizationCode') !== -1) {{
                        window.location.replace('http://127.0.0.1:{port}/auth?nonce={session_nonce}&data=' + encodeURIComponent(pText));
                        return;
                    }}
                }}
            }} catch(e) {{}}
        }})();
    "#);

    let login_window = WebviewWindowBuilder::new(
        app,
        "epic-login",
        WebviewUrl::External(LOGIN_URL.parse().context("URL de login inválida")?),
    )
    .title("Login com a Epic Games")
    .inner_size(480.0, 720.0)
    .initialization_script(&init_script)
    .on_navigation(move |nav_url| {
        // 1. Verifica parâmetros diretos da URL
        for (k, v) in nav_url.query_pairs() {
            if (k == "code" || k == "authorizationCode") && !v.is_empty() {
                let mut lock = captured_for_nav.lock().unwrap();
                if lock.is_none() {
                    *lock = Some(v.to_string());
                }
                return false;
            }
        }

        // 2. Intercepta o redirecionamento local do script validando o nonce de sessão
        if nav_url.host_str() == Some("127.0.0.1") || nav_url.host_str() == Some("localhost") {
            let has_nonce = nav_url.query_pairs().any(|(k, v)| k == "nonce" && v == nonce_for_nav);
            if has_nonce {
                for (k, v) in nav_url.query_pairs() {
                    if (k == "data" || k == "code" || k == "authorizationCode") && !v.is_empty() {
                        let mut lock = captured_for_nav.lock().unwrap();
                        if lock.is_none() {
                            *lock = Some(v.to_string());
                        }
                        return false;
                    }
                }
            }
        }

        true
    })

    .build()
    .context("falha ao abrir a janela de login")?;

    login_window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                closed_flag.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
    });

    let _ = window.emit(
        "epic-log",
        serde_json::json!({ "line": "Janela de autenticação aberta. Aguardando login..." }),
    );

    // Faz polling leve disparando o script avaliado até capturar ou fechar
    for _ in 0..1200 {
        tokio::time::sleep(Duration::from_millis(250)).await;

        if closed.load(Ordering::SeqCst) {
            break;
        }

        if captured.lock().unwrap().is_some() {
            break;
        }

        let _ = login_window.eval(&eval_script);
    }

    let _ = login_window.close();

    let json_text = captured.lock().unwrap().clone().ok_or_else(|| {
        anyhow!("não consegui capturar o código automaticamente — tente colar o código manualmente")
    })?;

    let _ = window.emit(
        "epic-log",
        serde_json::json!({ "line": "Código de autorização capturado automaticamente!" }),
    );

    login_with_code(window, &json_text).await
}

/// Gera a URL oficial de Single Sign-On (SSO) da Epic para a página de download da Unreal Engine no Linux.
pub async fn get_sso_download_url() -> Result<String> {
    let access_token = get_valid_access_token().await?;
    let client = create_client()?;

    let exchange_resp = client
        .get("https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/exchange")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .context("falha ao gerar token de sessão SSO na Epic")?;

    if !exchange_resp.status().is_success() {
        return Err(anyhow!("a Epic rejeitou a solicitação de token SSO — tente fazer login novamente"));
    }

    let exchange_code_data: ExchangeCodeResponse = exchange_resp.json().await?;
    Ok(format!(
        "https://www.epicgames.com/id/exchange?exchangeCode={}&redirectUrl=https%3A%2F%2Fwww.unrealengine.com%2Flinux",
        exchange_code_data.code
    ))
}

pub fn is_valid_engine_download_url(u: &str) -> bool {
    let u = u.trim();
    if !u.starts_with("http://") && !u.starts_with("https://") {
        return false;
    }
    if u.contains('#') {
        return false;
    }
    if u.contains("127.0.0.1") || u.contains("localhost") {
        return false;
    }
    if u.contains("unrealengine.com/linux") || u.contains("epicgames.com/id/") {
        return false;
    }
    let path = u.split('?').next().unwrap_or(u).to_lowercase();
    path.ends_with(".zip")
        || u.contains(".zip?")
        || (u.contains(".zip") && u.contains("amazonaws.com"))
        || u.contains("ucs-blob-store")
}

pub fn extract_clean_zip_filename(url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        for (k, v) in parsed.query_pairs() {
            if k == "response-content-disposition" {
                if let Some(pos) = v.find("filename*=") {
                    let s = &v[pos + 10..];
                    let f = s.split("''").last().unwrap_or(s);
                    if !f.is_empty() {
                        return f.to_string();
                    }
                } else if let Some(pos) = v.find("filename=") {
                    let s = &v[pos + 9..];
                    let f = s.trim_matches('"').split(';').next().unwrap_or(s);
                    if !f.is_empty() {
                        return f.to_string();
                    }
                }
            }
        }
    }
    let last = url.split('?').next().unwrap_or(url).split('/').last().unwrap_or("Linux_Unreal_Engine.zip");
    if last.ends_with(".zip") {
        last.to_string()
    } else {
        format!("{last}.zip")
    }
}

/// Resolve a URL oficial do pacote pré-compilado da Unreal Engine via SSO de forma silenciosa,
/// capturando o link pré-assinado da AWS S3 do arquivo .zip oficial.
pub async fn open_engine_downloader_window(
    app: &AppHandle,
    _window: &Window,
    target_version: Option<String>,
) -> Result<Option<EngineBlob>> {
    let sso_url = get_sso_download_url().await?;

    // Fecha janela anterior se existir
    if let Some(existing) = app.get_webview_window("epic-download-portal") {
        let _ = existing.close();
    }

    let captured = Arc::new(Mutex::new(None::<EngineBlob>));
    let captured_nav = captured.clone();
    let captured_tcp = captured.clone();

    let listener = TcpListener::bind("127.0.0.1:0")
        .context("falha ao iniciar listener para interceptação de download")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();

    let closed = Arc::new(AtomicBool::new(false));
    let closed_flag = closed.clone();
    let closed_tcp = closed.clone();

    std::thread::spawn(move || {
        while !closed_tcp.load(Ordering::SeqCst) {
            if captured_tcp.lock().unwrap().is_some() {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 8192];
                    if let Ok(n) = stream.read(&mut buf) {
                        let req = String::from_utf8_lossy(&buf[..n]);
                        if let Some(first_line) = req.lines().next() {
                            if let Some(path) = first_line.split_whitespace().nth(1) {
                                if let Ok(parsed) = reqwest::Url::parse(&format!("http://127.0.0.1:{port}{path}")) {
                                    for (k, v) in parsed.query_pairs() {
                                        if k == "url" && is_valid_engine_download_url(&v) {
                                            let real_s3_url = v.to_string();
                                            let clean_name = extract_clean_zip_filename(&real_s3_url);
                                            println!("[DOWNLOAD INTERCEPTOR] Capturado link da S3 via TCP: {} ({})", &clean_name, &real_s3_url);
                                            let mut lock = captured_tcp.lock().unwrap();
                                            *lock = Some(EngineBlob {
                                                name: clean_name,
                                                url: real_s3_url,
                                                size: 0,
                                                created_at: String::new(),
                                                ..Default::default()
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\nOK";
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    });

    let target_ver_str = target_version.unwrap_or_default();

    // Script inteligente injetado: busca na API de blobs, intercepta rede e analisa botões no DOM
    let init_script = format!(r#"
        (function() {{
            var reported = false;
            function isRealZipUrl(u) {{
                if (!u || typeof u !== 'string') return false;
                u = u.trim();
                if (!u.startsWith('http://') && !u.startsWith('https://')) return false;
                if (u.indexOf('#') !== -1) return false;
                if (u.indexOf('unrealengine.com/linux') !== -1 || u.indexOf('epicgames.com/id/') !== -1) return false;
                var clean = u.split('?')[0].toLowerCase();
                if (clean.endsWith('.zip')) return true;
                if (u.indexOf('.zip?') !== -1 || (u.indexOf('.zip') !== -1 && u.indexOf('amazonaws.com') !== -1)) return true;
                return false;
            }}

            function report(url) {{
                if (reported || !isRealZipUrl(url)) return;
                reported = true;
                window.location.replace('http://127.0.0.1:{port}/download?url=' + encodeURIComponent(url));
            }}

            var targetVer = "{target_ver_str}".toLowerCase();
            var majorMinor = targetVer ? targetVer.split('.').slice(0, 2).join('.') : '';

            // 1. Consulta diretamente a API de blobs de Linux com os cookies autenticados do SSO
            async function tryFetchBlobs() {{
                try {{
                    var res = await fetch('https://www.unrealengine.com/api/blobs/linux', {{ credentials: 'include' }});
                    if (res.ok) {{
                        var data = await res.json();
                        if (data && data.blobs && data.blobs.length > 0) {{
                            for (var b of data.blobs) {{
                                if (isRealZipUrl(b.url)) {{
                                    var bname = (b.name || '').toLowerCase();
                                    if (targetVer && (bname.indexOf(targetVer) !== -1 || bname.indexOf(majorMinor) !== -1)) {{
                                        report(b.url);
                                        return true;
                                    }}
                                }}
                            }}
                            for (var b of data.blobs) {{
                                if (isRealZipUrl(b.url)) {{
                                    report(b.url);
                                    return true;
                                }}
                            }}
                        }}
                    }}
                }} catch(e) {{}}
                return false;
            }}

            // 2. Varre o DOM por links diretos .zip ou botões de download
            function scanDOM() {{
                // Auto aceita EULA se houver na tela
                var cbs = document.querySelectorAll('input[type="checkbox"]');
                for (var i = 0; i < cbs.length; i++) {{
                    if (!cbs[i].checked) {{
                        cbs[i].checked = true;
                        cbs[i].dispatchEvent(new Event('change', {{ bubbles: true }}));
                    }}
                }}

                var anchors = document.querySelectorAll('a');
                for (var i = 0; i < anchors.length; i++) {{
                    var h = anchors[i].href || '';
                    if (isRealZipUrl(h)) {{
                        if (!targetVer || h.toLowerCase().indexOf(targetVer) !== -1 || (majorMinor && h.toLowerCase().indexOf(majorMinor) !== -1)) {{
                            report(h);
                            return true;
                        }}
                    }}
                }}

                // Procura botões com download da versão e aciona clique
                var clickables = document.querySelectorAll('a, button');
                for (var j = 0; j < clickables.length; j++) {{
                    var el = clickables[j];
                    var txt = (el.innerText || el.textContent || '').toLowerCase();
                    if ((txt.indexOf('download') !== -1 || txt.indexOf('baixar') !== -1) && 
                        (!targetVer || txt.indexOf(targetVer) !== -1 || (majorMinor && txt.indexOf(majorMinor) !== -1))) {{
                        try {{ el.click(); }} catch(e) {{}}
                    }}
                }}
                return false;
            }}

            // Intercepta chamadas de fetch do site
            var origFetch = window.fetch;
            window.fetch = async function(...args) {{
                var res = await origFetch.apply(this, args);
                try {{
                    var clone = res.clone();
                    clone.json().then(data => {{
                        if (data && data.blobs && data.blobs.length > 0) {{
                            for (var b of data.blobs) {{
                                if (isRealZipUrl(b.url)) {{
                                    var bname = (b.name || '').toLowerCase();
                                    if (targetVer && (bname.indexOf(targetVer) !== -1 || (majorMinor && bname.indexOf(majorMinor) !== -1))) {{
                                        report(b.url);
                                        return;
                                    }}
                                }}
                            }}
                            if (data.blobs[0] && isRealZipUrl(data.blobs[0].url)) {{
                                report(data.blobs[0].url);
                            }}
                        }}
                    }}).catch(() => {{}});
                }} catch(e) {{}}
                return res;
            }};

            var interval = setInterval(async function() {{
                if (reported) {{
                    clearInterval(interval);
                    return;
                }}
                if (await tryFetchBlobs()) {{
                    clearInterval(interval);
                    return;
                }}
                if (scanDOM()) {{
                    clearInterval(interval);
                    return;
                }}
            }}, 350);

            setTimeout(function() {{
                clearInterval(interval);
            }}, 40000);
        }})();
    "#);

    // Inicia a janela 100% invisível em background fora da tela
    let dl_window = WebviewWindowBuilder::new(
        app,
        "epic-download-portal",
        WebviewUrl::External(sso_url.parse().context("URL de SSO inválida")?),
    )
    .title("Epic Games Engine Resolver")
    .position(-9999.0, -9999.0)
    .inner_size(800.0, 600.0)
    .visible(false)
    .skip_taskbar(true)
    .initialization_script(&init_script)
    .on_navigation(move |nav_url| {
        // 1. Se for o redirecionamento local para 127.0.0.1, extrai o parâmetro "url"
        if nav_url.host_str() == Some("127.0.0.1") || nav_url.host_str() == Some("localhost") {
            for (k, v) in nav_url.query_pairs() {
                if k == "url" && is_valid_engine_download_url(&v) {
                    let real_s3_url = v.to_string();
                    let clean_name = extract_clean_zip_filename(&real_s3_url);
                    println!("[DOWNLOAD INTERCEPTOR] Capturado link da S3 via 127.0.0.1: {} ({})", &clean_name, &real_s3_url);
                    let mut lock = captured_nav.lock().unwrap();
                    *lock = Some(EngineBlob {
                        name: clean_name,
                        url: real_s3_url,
                        size: 0,
                        created_at: String::new(),
                        ..Default::default()
                    });
                    return false;
                }
            }
            return false;
        }

        // 2. Se for navegação direta para a AWS S3
        let s = nav_url.as_str();
        if is_valid_engine_download_url(s) {
            let clean_name = extract_clean_zip_filename(s);
            println!("[DOWNLOAD INTERCEPTOR] Capturado link da S3 direto via navegação: {} ({})", &clean_name, s);
            let mut lock = captured_nav.lock().unwrap();
            *lock = Some(EngineBlob {
                name: clean_name,
                url: s.to_string(),
                size: 0,
                created_at: String::new(),
                ..Default::default()
            });
            return false;
        }

        true
    })
    .build()
    .context("falha ao inicializar o capturador de download da Epic")?;

    dl_window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                closed_flag.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
    });

    for _ in 0..1200 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if closed.load(Ordering::SeqCst) {
            break;
        }
        if captured.lock().unwrap().is_some() {
            break;
        }
    }

    let res = captured.lock().unwrap().clone();
    let _ = dl_window.close();
    Ok(res)
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct EpicAssetItem {
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    #[serde(rename = "buildVersion")]
    pub build_version: Option<String>,
    #[serde(rename = "catalogItemId")]
    pub catalog_item_id: Option<String>,
}

fn estimate_engine_size(ver: &str) -> u64 {
    if ver.starts_with("5.5") {
        34_500_000_000
    } else if ver.starts_with("5.4") {
        32_800_000_000
    } else if ver.starts_with("5.3") {
        28_700_000_000
    } else if ver.starts_with("5.2") {
        27_900_000_000
    } else if ver.starts_with("5.1") {
        26_400_000_000
    } else if ver.starts_with("5.0") {
        24_800_000_000
    } else if ver.starts_with("4.27") {
        18_300_000_000
    } else if ver.starts_with("4.26") {
        16_000_000_000
    } else {
        12_000_000_000
    }
}

fn sort_version_desc(a: &str, b: &str) -> std::cmp::Ordering {
    let parse_nums = |s: &str| -> Vec<u32> {
        s.split('.')
            .filter_map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok())
            .collect()
    };
    let a_nums = parse_nums(a);
    let b_nums = parse_nums(b);
    b_nums.cmp(&a_nums)
}

fn fallback_engine_catalog() -> Vec<EngineBlob> {
    let versions = [
        ("5.5.4", true), ("5.4.4", true), ("5.3.2", true), ("5.2.1", true),
        ("5.1.1", true), ("5.0.3", true), ("4.27.2", false), ("4.26.2", false),
        ("4.25.4", false), ("4.24.3", false), ("4.23.1", false), ("4.22.3", false),
        ("4.21.2", false), ("4.20.3", false), ("4.19.2", false), ("4.18.3", false),
    ];

    versions
        .iter()
        .map(|(ver, precompiled)| {
            let name = if *precompiled {
                format!("Linux_Unreal_Engine_{ver}.zip")
            } else {
                format!("Unreal_Engine_{ver}_Source.zip")
            };
            EngineBlob {
                name,
                version: ver.to_string(),
                app_name: format!("UE_{}", ver.split('.').take(2).collect::<Vec<_>>().join(".")),
                is_precompiled: *precompiled,
                size: estimate_engine_size(ver),
                created_at: String::new(),
                url: String::new(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCatalogCache {
    pub updated_at: u64,
    pub engines: Vec<EngineBlob>,
}

fn catalog_cache_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("não consegui determinar o diretório de config"))?
        .join("unreal-launcher");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir.join("engine_catalog_cache.json"))
}

pub fn load_cached_engine_catalog() -> Option<Vec<EngineBlob>> {
    let path = catalog_cache_path().ok()?;
    load_cached_engine_catalog_from(&path)
}

pub fn load_cached_engine_catalog_from(path: &std::path::Path) -> Option<Vec<EngineBlob>> {
    if !path.exists() {
        return None;
    }
    let data = fs::read_to_string(path).ok()?;
    let cache: EngineCatalogCache = serde_json::from_str(&data).ok()?;
    if cache.engines.is_empty() {
        None
    } else {
        Some(cache.engines)
    }
}

pub fn save_cached_engine_catalog(engines: &[EngineBlob]) -> Result<()> {
    let path = catalog_cache_path()?;
    save_cached_engine_catalog_to(&path, engines)
}

pub fn save_cached_engine_catalog_to(path: &std::path::Path, engines: &[EngineBlob]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cache = EngineCatalogCache {
        updated_at: now,
        engines: engines.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)?;
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

/// Consulta o endpoint da Epic Games para obter todas as engines disponíveis para a conta.
pub async fn fetch_remote_engine_blobs() -> Result<Vec<EngineBlob>> {
    let access_token = get_valid_access_token().await?;
    let client = create_client()?;
    let url = "https://launcher-public-service-prod06.ol.epicgames.com/launcher/api/public/assets/Windows?label=Live";
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .context("falha ao conectar aos servidores da Epic Games")?;

    if !resp.status().is_success() {
        return Err(anyhow!("status de erro da Epic Games: {}", resp.status()));
    }

    let assets = resp
        .json::<Vec<EpicAssetItem>>()
        .await
        .context("falha ao processar lista de engines da Epic Games")?;

    let mut blobs = Vec::new();
    for a in assets {
        if let Some(app) = a.app_name {
            if app.starts_with("UE_") {
                let bv = a.build_version.unwrap_or_default();
                let ver = bv
                    .split('-')
                    .next()
                    .unwrap_or(&app)
                    .split('+')
                    .next()
                    .unwrap_or(&app)
                    .to_string();

                let is_precompiled = ver.starts_with("5.");
                let name = if is_precompiled {
                    format!("Linux_Unreal_Engine_{ver}.zip")
                } else {
                    format!("Unreal_Engine_{ver}_Source.zip")
                };

                let size = estimate_engine_size(&ver);

                blobs.push(EngineBlob {
                    name,
                    version: ver,
                    app_name: app,
                    is_precompiled,
                    size,
                    created_at: String::new(),
                    url: String::new(),
                });
            }
        }
    }

    if blobs.is_empty() {
        return Err(anyhow!("nenhuma engine retornada pela API da Epic"));
    }

    blobs.sort_by(|a, b| sort_version_desc(&a.version, &b.version));
    let _ = save_cached_engine_catalog(&blobs);
    Ok(blobs)
}

/// Retorna a lista de versões. Se `force_refresh` for falso e houver cache local, responde instantaneamente sem rede.
pub async fn list_engine_blobs(force_refresh: bool) -> Result<Vec<EngineBlob>> {
    if !force_refresh {
        if let Some(cached) = load_cached_engine_catalog() {
            return Ok(cached);
        }
    }

    match fetch_remote_engine_blobs().await {
        Ok(blobs) => Ok(blobs),
        Err(err) => {
            eprintln!("Falha ao buscar catálogo remoto da Epic: {err:#}");
            if let Some(cached) = load_cached_engine_catalog() {
                Ok(cached)
            } else {
                Ok(fallback_engine_catalog())
            }
        }
    }
}

/// Worker em background que verifica periodicamente se o catálogo da Epic foi atualizado
pub fn start_catalog_sync_worker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Aguarda 3 segundos após a inicialização para não concorrer com abertura de janelas
        tokio::time::sleep(Duration::from_secs(3)).await;

        loop {
            if get_valid_access_token().await.is_ok() {
                if let Ok(remote) = fetch_remote_engine_blobs().await {
                    let cache_changed = match load_cached_engine_catalog() {
                        Some(cached) => {
                            if cached.len() != remote.len() {
                                true
                            } else {
                                cached
                                    .iter()
                                    .zip(&remote)
                                    .any(|(c, r)| c.version != r.version || c.name != r.name)
                            }
                        }
                        None => true,
                    };

                    if cache_changed {
                        let _ = save_cached_engine_catalog(&remote);
                        let _ = app.emit("engine-catalog-updated", &remote);
                    }
                }
            }

            // Repete a verificação a cada 15 minutos
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_cache_save_and_load() {
        let temp_dir = std::env::temp_dir().join(format!("test_engine_cat_{}", uuid::Uuid::new_v4()));
        let test_cache_path = temp_dir.join("engine_catalog_cache.json");

        let sample = vec![
            EngineBlob {
                name: "Linux_Unreal_Engine_5.5.4.zip".into(),
                version: "5.5.4".into(),
                app_name: "UE_5.5".into(),
                is_precompiled: true,
                size: 34_500_000_000,
                created_at: String::new(),
                url: String::new(),
            }
        ];

        assert!(save_cached_engine_catalog_to(&test_cache_path, &sample).is_ok());
        let loaded = load_cached_engine_catalog_from(&test_cache_path);
        assert!(loaded.is_some());
        let blobs = loaded.unwrap();
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].version, "5.5.4");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_is_valid_engine_download_url() {
        // Redirecionamento local NUNCA deve ser considerado URL de download
        assert!(!is_valid_engine_download_url("http://127.0.0.1:43221/download?url=https%3A%2F%2Fucs-blob-store.s3-accelerate.amazonaws.com%2Fblobs%2Ffile.zip"));
        assert!(!is_valid_engine_download_url("http://localhost:5000/download?url=https://s3.amazonaws.com/test.zip"));

        // Páginas HTML e âncoras NUNCA devem ser consideradas
        assert!(!is_valid_engine_download_url("https://www.unrealengine.com/linux#Linux_Unreal_Engine_5.5"));
        assert!(!is_valid_engine_download_url("https://www.unrealengine.com/linux"));

        // URLs da AWS S3 oficiais da Epic DEVEM ser válidas
        assert!(is_valid_engine_download_url("https://ucs-blob-store.s3-accelerate.amazonaws.com/blobs/d2/78/8d9a-5460?x=1&response-content-disposition=inline%3Bfilename%3D%22file.zip%22"));
        assert!(is_valid_engine_download_url("https://epicgames-engine-builds.s3.amazonaws.com/Linux_Unreal_Engine_5.5.4.zip?AWSAccessKeyId=123"));
    }

    #[test]
    fn test_extract_clean_zip_filename() {
        let u = "https://ucs-blob-store.s3-accelerate.amazonaws.com/blobs/d2/78/8d9a-5460?response-content-disposition=inline%3Bfilename%3D%22file.zip%22%3Bfilename%2A%3DUTF-8%27%27Linux_Unreal_Engine_5.8.2.zip&x-id=GetObject";
        assert_eq!(extract_clean_zip_filename(u), "Linux_Unreal_Engine_5.8.2.zip");

        let u2 = "https://epicgames-engine-builds.s3.amazonaws.com/Linux_Unreal_Engine_5.5.4.zip?AWSAccessKeyId=123";
        assert_eq!(extract_clean_zip_filename(u2), "Linux_Unreal_Engine_5.5.4.zip");
    }
}

