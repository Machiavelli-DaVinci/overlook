//! Google sign-in for installed apps: system browser, PKCE, loopback redirect.
//!
//! The UI starts a [`PendingSignIn`], opens its URL in the system browser, and waits on a
//! background thread for the browser to hit the loopback listener. The listener answers with
//! a small branded page and hands the authorization code back for the token exchange.

use overlook_provider::gmail::oauth::{exchange_code, fetch_userinfo, AuthorizationRequest, ClientConfig, Pkce, UserInfo};
use overlook_provider::gmail::OAuthTokens;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);

pub fn client_config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("OVERLOOK_CONFIG_DIR") {
        return PathBuf::from(dir).join("google-oauth-client.json");
    }
    // directories maps ("ai", "Last Refuge", "Overlook") to ~/.config/overlook on Linux.
    directories::ProjectDirs::from("ai", "Last Refuge", "Overlook")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".config/overlook"))
        .join("google-oauth-client.json")
}

/// Reads the OAuth client the user created in Google Cloud.
pub fn load_client_config() -> Result<ClientConfig, String> {
    let path = client_config_path();
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "No Google OAuth client found. Save the JSON file downloaded from Google Cloud as {}.",
            path.display()
        )
    })?;
    ClientConfig::from_google_json(&text).map_err(|e| format!("{} could not be read: {e}", path.display()))
}

pub struct SignInResult {
    pub tokens: OAuthTokens,
    pub user: UserInfo,
}

pub struct PendingSignIn {
    pub url: String,
    request: AuthorizationRequest,
    config: ClientConfig,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<String, String>>,
}

impl PendingSignIn {
    /// Binds the loopback listener and builds the authorization URL.
    pub fn start(config: ClientConfig, login_hint: Option<String>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("could not open a local port for sign-in: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let request = AuthorizationRequest {
            client_id: config.client_id.clone(),
            redirect_uri,
            state: Pkce::random_state(),
            pkce: Pkce::random(),
            login_hint,
        };
        let url = request.url();
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("overlook-oauth-callback".into())
            .spawn(move || {
                let started = Instant::now();
                loop {
                    if flag.load(Ordering::SeqCst) {
                        let _ = sender.send(Err("Sign-in was cancelled.".into()));
                        return;
                    }
                    if started.elapsed() > SIGN_IN_TIMEOUT {
                        let _ = sender.send(Err("Sign-in timed out. Try again when you are ready.".into()));
                        return;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Some(query) = handle_connection(stream) {
                                let _ = sender.send(Ok(query));
                                return;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(80)),
                        Err(e) => {
                            let _ = sender.send(Err(format!("the sign-in listener failed: {e}")));
                            return;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { url, request, config, cancel, receiver })
    }

    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Waits for the browser callback, then exchanges the code and reads the user's email.
    /// Blocking; run on a background thread.
    pub fn wait(self) -> Result<SignInResult, String> {
        let query = self.receiver.recv().map_err(|_| "the sign-in listener stopped unexpectedly".to_owned())??;
        let code = self.request.parse_callback(&query).map_err(|e| e.to_string())?;
        let tokens = exchange_code(&self.config, &code, &self.request.redirect_uri, &self.request.pkce.verifier).map_err(|e| e.to_string())?;
        if tokens.refresh_token.is_none() {
            return Err("Google did not return a refresh token. Remove Overlook from your Google account's third-party access and sign in again.".into());
        }
        let user = fetch_userinfo(&tokens.access_token).map_err(|e| e.to_string())?;
        Ok(SignInResult { tokens, user })
    }
}

/// Reads one HTTP request, answers with the completion page, and returns the query string
/// of a `/callback` request. Other paths (favicon) get a 404 and `None`.
fn handle_connection(mut stream: TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).ok()?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next()?;
    let target = first_line.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let denied = query.contains("error=");
    let body = completion_page(denied);
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{}",
            body.len(),
            body
        )
        .as_bytes(),
    );
    let _ = stream.flush();
    Some(query.to_owned())
}

fn completion_page(denied: bool) -> String {
    let (title, text) = if denied {
        ("Sign-in cancelled", "You can close this tab and return to Overlook.")
    } else {
        ("You’re signed in", "You can close this tab and return to Overlook. Your mail is on its way.")
    };
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Overlook · {title}</title>
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#101C24;color:#F0EBDD;font:16px/1.5 Inter,system-ui,sans-serif}}
main{{text-align:center;padding:40px}}h1{{font:400 40px/1.1 'Source Serif 4','Source Serif Pro',Georgia,serif;margin:18px 0 10px}}p{{color:#C6D5D5;margin:0}}
svg{{display:block;margin:0 auto}}small{{display:block;margin-top:34px;color:#A8B7BA;letter-spacing:.14em;font-size:12px}}</style></head>
<body><main><svg width="72" height="72" viewBox="0 0 128 128" aria-hidden="true"><circle cx="64" cy="64" r="49" fill="none" stroke="#F0EBDD" stroke-width="10"/><path d="M 21 79 L 48 59 L 72 77 L 92 64 L 109 78" fill="none" stroke="#F0EBDD" stroke-width="8" stroke-linejoin="round" stroke-linecap="round"/><circle cx="79" cy="42" r="5" fill="#FF9D32"/></svg>
<h1>{title}</h1><p>{text}</p><small>OVERLOOK BY LAST REFUGE</small></main></body></html>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_is_under_dot_config_overlook() {
        let path = client_config_path();
        assert!(path.ends_with(".config/overlook/google-oauth-client.json") || path.ends_with("overlook/google-oauth-client.json"), "{}", path.display());
    }

    #[test]
    fn completion_page_mentions_overlook() {
        assert!(completion_page(false).contains("You’re signed in"));
        assert!(completion_page(true).contains("cancelled"));
    }
}
