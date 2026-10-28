//! OAuth 2.0 loopback server and token exchange.
//!
//! Spawns a temporary TCP server on `127.0.0.1:53682` (or OAUTH_LOOPBACK_PORT),
//! generates PKCE credentials, and opens the system browser to authorize.
//! When redirected back, it serves a success page, completes token exchange,
//! and persists credentials in the OS keychain.

use std::time::Duration;

use oauth2::{CsrfToken, PkceCodeChallenge, PkceCodeVerifier};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::auth::keychain::{self, AuthTokens, SERVICE_NAME};
use crate::error::{AppError, Result};
use crate::types::SourceKind;

pub const DEFAULT_LOOPBACK_PORT: u16 = 53682;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    Microsoft,
    Notion,
}

impl OAuthProvider {
    pub fn as_source_kind(&self) -> SourceKind {
        match self {
            Self::Microsoft => SourceKind::OneDrive,
            Self::Notion => SourceKind::Notion,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthSuccess {
    pub provider: OAuthProvider,
    pub account_id: String,
    pub account_name: Option<String>,
}

/// Discovers the loopback port to bind.
pub fn loopback_port() -> u16 {
    std::env::var("OAUTH_LOOPBACK_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_LOOPBACK_PORT)
}

/// Response returned to the browser after callback.
pub fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|e| AppError::Other(format!("Failed to open browser: {e}")))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| AppError::Other(format!("Failed to open browser: {e}")))?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| AppError::Other(format!("Failed to open browser: {e}")))?;
    }
    Ok(())
}

fn form_encode(s: &str) -> String {
    let mut encoded = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}

const HTML_SUCCESS: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Authentication Successful</title></head>
<body style="font-family: system-ui, -apple-system, sans-serif; text-align: center; padding: 60px; background: #fafafa; color: #111;">
  <h2 style="color: #7c3aed;">Authentication Successful</h2>
  <p>You can close this tab and return to <strong>Native Desktop Search</strong>.</p>
</body>
</html>"#;

/// Starts the OAuth flow for `provider`:
/// 1. Prepares PKCE + state challenge
/// 2. Starts temporary listener
/// 3. Returns the authorization URL to open in the browser
/// 4. Awaits the callback, completes token exchange, and stores tokens in keychain
pub async fn run_oauth_flow(provider: OAuthProvider) -> Result<OAuthSuccess> {
    let port = loopback_port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let csrf_state = CsrfToken::new_random();

    let auth_url = build_auth_url(provider, &redirect_uri, &csrf_state, &pkce_challenge)?;

    tracing::info!(
        provider = ?provider,
        port,
        "Starting temporary OAuth loopback listener"
    );

    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .map_err(|e| {
            AppError::Other(format!(
                "Could not bind OAuth loopback server on port {port}: {e}"
            ))
        })?;

    // Open browser with auth_url
    if let Err(e) = open_browser(&auth_url) {
        tracing::warn!("Could not launch browser automatically: {e}. URL: {auth_url}");
    }

    // Wait for redirect callback with timeout (120 seconds)
    let callback_res = tokio::time::timeout(
        Duration::from_secs(120),
        handle_loopback_connection(listener, csrf_state.secret()),
    )
    .await
    .map_err(|_| AppError::Other("OAuth callback timed out after 2 minutes".into()))??;

    // Exchange auth code for tokens
    let tokens = exchange_code(provider, &callback_res.code, &pkce_verifier, &redirect_uri).await?;

    // Store in OS keychain
    let account_id = tokens.account_id.clone();
    let account_name = tokens.account_email.clone();
    let service_key = match provider {
        OAuthProvider::Microsoft => format!("microsoft:{account_id}"),
        OAuthProvider::Notion => format!("notion:{account_id}"),
    };

    keychain::save_tokens(SERVICE_NAME, &service_key, &tokens)?;
    tracing::info!(account_id = %account_id, "OAuth tokens stored securely in keychain");

    Ok(OAuthSuccess {
        provider,
        account_id,
        account_name,
    })
}

pub fn build_auth_url(
    provider: OAuthProvider,
    redirect_uri: &str,
    csrf_state: &CsrfToken,
    pkce_challenge: &PkceCodeChallenge,
) -> Result<String> {
    match provider {
        OAuthProvider::Microsoft => {
            let client_id = std::env::var("MICROSOFT_CLIENT_ID")
                .unwrap_or_else(|_| "mock_microsoft_client_id".into());
            let tenant = std::env::var("MICROSOFT_TENANT").unwrap_or_else(|_| "common".into());

            let base_url = if let Ok(mock_base) = std::env::var("MICROSOFT_API_BASE_URL") {
                format!("{mock_base}/common/oauth2/v2.0/authorize")
            } else {
                format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize")
            };

            let scopes = "offline_access Files.Read.All User.Read";
            Ok(format!(
                "{base_url}?client_id={client_id}&response_type=code&redirect_uri={redirect_uri}&response_mode=query&scope={scopes}&state={state}&code_challenge={challenge}&code_challenge_method=S256",
                state = csrf_state.secret(),
                challenge = pkce_challenge.as_str()
            ))
        }
        OAuthProvider::Notion => {
            let client_id = std::env::var("NOTION_CLIENT_ID")
                .unwrap_or_else(|_| "mock_notion_client_id".into());

            let base_url = if let Ok(mock_base) = std::env::var("NOTION_API_BASE_URL") {
                format!("{mock_base}/v1/oauth/authorize")
            } else {
                "https://api.notion.com/v1/oauth/authorize".into()
            };

            Ok(format!(
                "{base_url}?client_id={client_id}&response_type=code&owner=user&redirect_uri={redirect_uri}&state={state}",
                state = csrf_state.secret()
            ))
        }
    }
}

struct CallbackParams {
    code: String,
}

async fn handle_loopback_connection(
    listener: TcpListener,
    expected_state: &str,
) -> Result<CallbackParams> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        let mut buffer = [0u8; 2048];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);

        if let Some(first_line) = request_str.lines().next() {
            if first_line.starts_with("GET /favicon.ico") {
                let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
                continue;
            }

            if first_line.starts_with("GET /callback") {
                let (code, state) = parse_callback_params(first_line)?;

                if state != expected_state {
                    let err_body = "CSRF state token mismatch";
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                        err_body.len(),
                        err_body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    return Err(AppError::Other(
                        "CSRF state mismatch during OAuth authentication".into(),
                    ));
                }

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n{}",
                    HTML_SUCCESS.len(),
                    HTML_SUCCESS
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;

                return Ok(CallbackParams { code });
            }
        }
    }
}

pub fn parse_callback_params(line: &str) -> Result<(String, String)> {
    let path = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::Other("Malformed HTTP callback request".into()))?;

    let query_str = path
        .split('?')
        .nth(1)
        .ok_or_else(|| AppError::Other("Missing query parameters in OAuth callback".into()))?;

    let mut code = None;
    let mut state = None;

    for pair in query_str.split('&') {
        let mut parts = pair.split('=');
        let key = parts.next().unwrap_or("");
        let val = parts.next().unwrap_or("");
        if key == "code" {
            code = Some(val.to_string());
        } else if key == "state" {
            state = Some(val.to_string());
        }
    }

    match (code, state) {
        (Some(c), Some(s)) => Ok((c, s)),
        _ => Err(AppError::Other(
            "Callback missing 'code' or 'state' parameters".into(),
        )),
    }
}

pub async fn exchange_code(
    provider: OAuthProvider,
    code: &str,
    pkce_verifier: &PkceCodeVerifier,
    redirect_uri: &str,
) -> Result<AuthTokens> {
    let client = reqwest::Client::new();

    match provider {
        OAuthProvider::Microsoft => {
            let client_id = std::env::var("MICROSOFT_CLIENT_ID")
                .unwrap_or_else(|_| "mock_microsoft_client_id".into());
            let tenant = std::env::var("MICROSOFT_TENANT").unwrap_or_else(|_| "common".into());

            let token_url = if let Ok(mock_base) = std::env::var("MICROSOFT_API_BASE_URL") {
                format!("{mock_base}/microsoft/token")
            } else {
                format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token")
            };

            let params = [
                ("client_id", client_id.as_str()),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", pkce_verifier.secret().as_str()),
            ];

            let body_str = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, form_encode(v)))
                .collect::<Vec<_>>()
                .join("&");

            let resp = client
                .post(&token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body_str)
                .send()
                .await
                .map_err(|e| AppError::Other(format!("Token request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(AppError::Other(format!(
                    "Token exchange failed ({status}): {text}"
                )));
            }

            #[derive(Deserialize)]
            struct MsTokenResponse {
                access_token: String,
                refresh_token: Option<String>,
                expires_in: Option<i64>,
            }

            let data: MsTokenResponse = resp
                .json()
                .await
                .map_err(|e| AppError::Other(format!("Invalid token JSON: {e}")))?;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            // Fetch user profile to get account ID / email
            let graph_base = std::env::var("MICROSOFT_API_BASE_URL")
                .unwrap_or_else(|_| "https://graph.microsoft.com".into());
            let me_url = format!("{graph_base}/v1.0/me");

            let (account_id, email) = if let Ok(me_resp) = client
                .get(&me_url)
                .bearer_auth(&data.access_token)
                .send()
                .await
            {
                #[derive(Deserialize)]
                struct Me {
                    id: Option<String>,
                    mail: Option<String>,
                    user_principal_name: Option<String>,
                }
                let me: Me = me_resp.json().await.unwrap_or(Me {
                    id: None,
                    mail: None,
                    user_principal_name: None,
                });
                let id = me.id.unwrap_or_else(|| "ms_account".into());
                let email = me.mail.or(me.user_principal_name);
                (id, email)
            } else {
                ("ms_account".into(), None)
            };

            Ok(AuthTokens {
                access_token: data.access_token,
                refresh_token: data.refresh_token,
                expires_at: data.expires_in.map(|secs| now + secs),
                account_id,
                account_email: email,
            })
        }
        OAuthProvider::Notion => {
            let client_id = std::env::var("NOTION_CLIENT_ID")
                .unwrap_or_else(|_| "mock_notion_client_id".into());
            let client_secret = std::env::var("NOTION_CLIENT_SECRET")
                .unwrap_or_else(|_| "mock_notion_client_secret".into());

            let token_url = if let Ok(mock_base) = std::env::var("NOTION_API_BASE_URL") {
                format!("{mock_base}/v1/oauth/token")
            } else {
                "https://api.notion.com/v1/oauth/token".into()
            };

            #[derive(Serialize)]
            struct NotionTokenReq<'a> {
                grant_type: &'a str,
                code: &'a str,
                redirect_uri: &'a str,
            }

            let req_body = NotionTokenReq {
                grant_type: "authorization_code",
                code,
                redirect_uri,
            };

            let resp = client
                .post(&token_url)
                .basic_auth(client_id, Some(client_secret))
                .header("Notion-Version", "2022-06-28")
                .json(&req_body)
                .send()
                .await
                .map_err(|e| AppError::Other(format!("Notion token request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(AppError::Other(format!(
                    "Notion token exchange failed ({status}): {text}"
                )));
            }

            #[derive(Deserialize)]
            struct NotionTokenResp {
                access_token: String,
                workspace_id: String,
                workspace_name: Option<String>,
            }

            let data: NotionTokenResp = resp
                .json()
                .await
                .map_err(|e| AppError::Other(format!("Invalid Notion JSON response: {e}")))?;

            Ok(AuthTokens {
                access_token: data.access_token,
                refresh_token: None,
                expires_at: None, // Notion access tokens do not expire
                account_id: data.workspace_id,
                account_email: data.workspace_name,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_callback_params_succeeds() {
        let line = "GET /callback?code=abc123xyz&state=state_secret_456 HTTP/1.1";
        let (code, state) = parse_callback_params(line).unwrap();
        assert_eq!(code, "abc123xyz");
        assert_eq!(state, "state_secret_456");
    }

    #[test]
    fn build_auth_url_contains_redirect_and_state() {
        let (challenge, _) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random();

        let ms_url = build_auth_url(
            OAuthProvider::Microsoft,
            "http://127.0.0.1:53682/callback",
            &state,
            &challenge,
        )
        .unwrap();
        assert!(ms_url.contains("client_id="));
        assert!(ms_url.contains("redirect_uri=http://127.0.0.1:53682/callback"));
        assert!(ms_url.contains(state.secret()));

        let notion_url = build_auth_url(
            OAuthProvider::Notion,
            "http://127.0.0.1:53682/callback",
            &state,
            &challenge,
        )
        .unwrap();
        assert!(notion_url.contains("client_id="));
        assert!(notion_url.contains(state.secret()));
    }
}
