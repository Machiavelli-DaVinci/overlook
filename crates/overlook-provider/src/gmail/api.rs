//! Authenticated Gmail REST client (blocking; call from a background thread).

use super::oauth::{self, url_encode, AuthError, ClientConfig, OAuthTokens};
use crate::ProviderError;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    NotFound,
    Unauthorized,
    RateLimited(String),
    Server(String),
    Client(u16, String),
    Transport(String),
}

impl From<ApiError> for ProviderError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Transport(_) => ProviderError::Offline,
            ApiError::Unauthorized => ProviderError::SignInRequired,
            ApiError::RateLimited(m) => ProviderError::RateLimited(m),
            ApiError::Server(m) => ProviderError::Temporary(m),
            ApiError::NotFound => ProviderError::Rejected("not found at Gmail".into()),
            ApiError::Client(code, m) => ProviderError::Rejected(format!("Gmail returned {code}: {m}")),
        }
    }
}

pub type TokenSink = Box<dyn Fn(&OAuthTokens) + Send + Sync>;

/// Minimum spacing between requests. Gmail meters usage in quota units per user per minute;
/// a `messages.get` costs 5 units, so 6 requests per second stays well inside the default
/// allowance while keeping a 150-message download under half a minute.
pub const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(170);

pub struct GmailClient {
    config: ClientConfig,
    tokens: Mutex<OAuthTokens>,
    on_tokens: TokenSink,
    agent: ureq::Agent,
    last_request: Mutex<Option<Instant>>,
}

impl GmailClient {
    /// `on_tokens` is called whenever tokens are refreshed so the caller can persist them.
    pub fn new(config: ClientConfig, tokens: OAuthTokens, on_tokens: TokenSink) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(40)))
            .build()
            .into();
        Self { config, tokens: Mutex::new(tokens), on_tokens, agent, last_request: Mutex::new(None) }
    }

    fn throttle(&self) {
        let mut last = self.last_request.lock().unwrap();
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < MIN_REQUEST_INTERVAL {
                std::thread::sleep(MIN_REQUEST_INTERVAL - elapsed);
            }
        }
        *last = Some(Instant::now());
    }

    pub fn tokens(&self) -> OAuthTokens {
        self.tokens.lock().unwrap().clone()
    }

    fn access_token(&self, force_refresh: bool) -> Result<String, ApiError> {
        let current = self.tokens();
        if !force_refresh && !current.is_expired(overlook_core::time::now()) {
            return Ok(current.access_token);
        }
        let Some(refresh) = current.refresh_token.as_deref() else {
            return Err(ApiError::Unauthorized);
        };
        match oauth::refresh(&self.config, refresh) {
            Ok(fresh) => {
                (self.on_tokens)(&fresh);
                let token = fresh.access_token.clone();
                *self.tokens.lock().unwrap() = fresh;
                Ok(token)
            }
            Err(AuthError::Denied(_)) => Err(ApiError::Unauthorized),
            Err(AuthError::Network(m)) => Err(ApiError::Transport(m)),
            Err(AuthError::Protocol(m)) => Err(ApiError::Server(m)),
        }
    }

    fn url(path: &str, query: &[(&str, &str)]) -> String {
        let mut url = format!("{API_BASE}/{path}");
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect::<Vec<_>>().join("&"));
        }
        url
    }

    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, ApiError> {
        self.with_retry(|token| {
            self.agent
                .get(&Self::url(path, query))
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .map_err(|e| ApiError::Transport(e.to_string()))
        })
    }

    pub fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.with_retry(|token| {
            self.agent
                .post(&Self::url(path, &[]))
                .header("Authorization", &format!("Bearer {token}"))
                .send_json(body)
                .map_err(|e| ApiError::Transport(e.to_string()))
        })
    }

    /// Runs the request, refreshing the token once on 401 and mapping statuses to errors.
    fn with_retry(
        &self,
        request: impl Fn(&str) -> Result<ureq::http::Response<ureq::Body>, ApiError>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut token = self.access_token(false)?;
        for attempt in 0..2 {
            self.throttle();
            let mut response = request(&token)?;
            let status = response.status().as_u16();
            let text = response.body_mut().read_to_string().map_err(|e| ApiError::Transport(e.to_string()))?;
            match status {
                200..=299 => {
                    if text.trim().is_empty() {
                        return Ok(serde_json::Value::Null);
                    }
                    return serde_json::from_str(&text).map_err(|e| ApiError::Server(format!("invalid JSON from Gmail: {e}")));
                }
                401 if attempt == 0 => {
                    token = self.access_token(true)?;
                    continue;
                }
                401 => return Err(ApiError::Unauthorized),
                404 => return Err(ApiError::NotFound),
                403 | 429 => {
                    let message = api_message(&text);
                    let lower = message.to_ascii_lowercase();
                    if status == 429 || lower.contains("ratelimit") || lower.contains("quota") || lower.contains("rate limit") {
                        return Err(ApiError::RateLimited(message));
                    }
                    return Err(ApiError::Client(status, message));
                }
                500..=599 => return Err(ApiError::Server(api_message(&text))),
                _ => return Err(ApiError::Client(status, api_message(&text))),
            }
        }
        Err(ApiError::Unauthorized)
    }
}

fn api_message(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(200).collect())
}
