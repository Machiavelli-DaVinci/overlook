//! Token storage in the system keyring (Secret Service, or the secrets portal in Flatpak).
//!
//! Tokens never touch the SQLite database, configuration files, or logs. When the keyring
//! is unavailable or locked the error is surfaced to the user instead of falling back to a
//! plain file.

use overlook_core::AccountId;
use overlook_provider::gmail::OAuthTokens;
use std::collections::HashMap;
use std::sync::OnceLock;

const APPLICATION: &str = "ai.lastrefuge.Overlook";

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("overlook-keyring")
            .build()
            .expect("tokio runtime")
    })
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum VaultError {
    #[error("The system keyring is not available: {0}")]
    Unavailable(String),
    #[error("Stored sign-in data could not be read: {0}")]
    Corrupt(String),
}

fn attributes(account: &AccountId) -> HashMap<&'static str, String> {
    HashMap::from([("application", APPLICATION.to_owned()), ("account", account.0.clone()), ("kind", "oauth-tokens".to_owned())])
}

fn borrow<'a>(attrs: &'a HashMap<&'static str, String>) -> HashMap<&'a str, &'a str> {
    attrs.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// Saves tokens for an account, replacing any previous entry. Blocking.
pub fn save_tokens(account: &AccountId, tokens: &OAuthTokens) -> Result<(), VaultError> {
    let attrs = attributes(account);
    let secret = serde_json::to_vec(tokens).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        keyring
            .create_item(&format!("Overlook Gmail sign-in ({})", account.0), &borrow(&attrs), secret, true)
            .await
            .map_err(|e| VaultError::Unavailable(e.to_string()))
    })
}

/// Loads tokens for an account. `Ok(None)` when nothing is stored. Blocking.
pub fn load_tokens(account: &AccountId) -> Result<Option<OAuthTokens>, VaultError> {
    let attrs = attributes(account);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        let items = keyring.search_items(&borrow(&attrs)).await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        let Some(item) = items.into_iter().next() else { return Ok(None) };
        let secret = item.secret().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        serde_json::from_slice(secret.as_bytes()).map(Some).map_err(|e| VaultError::Corrupt(e.to_string()))
    })
}

/// Deletes stored tokens for an account. Blocking.
pub fn delete_tokens(account: &AccountId) -> Result<(), VaultError> {
    let attrs = attributes(account);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        keyring.delete(&borrow(&attrs)).await.map_err(|e| VaultError::Unavailable(e.to_string()))
    })
}
