//! Secret storage in the system keyring (Secret Service, or the secrets portal in Flatpak).
//!
//! Secrets never touch the SQLite database, configuration files, or logs. When the keyring
//! is unavailable or locked the error is surfaced to the user instead of falling back to a
//! plain file.
//!
//! Each secret is one keyring item with the attributes `(application, account, kind)`:
//! Gmail accounts hold OAuth tokens under [`KIND_OAUTH_TOKENS`]; IMAP and POP3 accounts
//! hold their passwords under [`KIND_PASSWORD_INCOMING`] and, unless the outgoing server
//! shares it, [`KIND_PASSWORD_OUTGOING`].

use overlook_core::AccountId;
use overlook_provider::gmail::OAuthTokens;
use std::collections::HashMap;
use std::sync::OnceLock;

const APPLICATION: &str = "ai.lastrefuge.Overlook";

/// Gmail OAuth tokens, JSON-encoded.
pub const KIND_OAUTH_TOKENS: &str = "oauth-tokens";
/// IMAP or POP3 password, UTF-8.
pub const KIND_PASSWORD_INCOMING: &str = "password:incoming";
/// SMTP password, UTF-8. Absent when "same as incoming" is ticked.
pub const KIND_PASSWORD_OUTGOING: &str = "password:outgoing";

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

fn attributes(account: &AccountId, kind: &str) -> HashMap<&'static str, String> {
    HashMap::from([("application", APPLICATION.to_owned()), ("account", account.0.clone()), ("kind", kind.to_owned())])
}

/// Attributes shared by every secret of an account, for deleting them all at once.
fn account_attributes(account: &AccountId) -> HashMap<&'static str, String> {
    HashMap::from([("application", APPLICATION.to_owned()), ("account", account.0.clone())])
}

fn borrow<'a>(attrs: &'a HashMap<&'static str, String>) -> HashMap<&'a str, &'a str> {
    attrs.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// The item label shown in keyring managers.
fn label(account: &AccountId, kind: &str) -> String {
    let what = match kind {
        KIND_OAUTH_TOKENS => "Gmail sign-in",
        KIND_PASSWORD_INCOMING => "mail server password",
        KIND_PASSWORD_OUTGOING => "outgoing server password",
        other => other,
    };
    format!("Overlook {what} ({})", account.0)
}

/// Saves a secret of `kind` for an account, replacing any previous entry. Blocking.
pub fn save_secret(account: &AccountId, kind: &str, secret: Vec<u8>) -> Result<(), VaultError> {
    let attrs = attributes(account, kind);
    let label = label(account, kind);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        keyring.create_item(&label, &borrow(&attrs), secret, true).await.map_err(|e| VaultError::Unavailable(e.to_string()))
    })
}

/// Loads a secret of `kind` for an account. `Ok(None)` when nothing is stored. Blocking.
pub fn load_secret(account: &AccountId, kind: &str) -> Result<Option<Vec<u8>>, VaultError> {
    let attrs = attributes(account, kind);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        let items = keyring.search_items(&borrow(&attrs)).await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        let Some(item) = items.into_iter().next() else { return Ok(None) };
        let secret = item.secret().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        Ok(Some(secret.as_bytes().to_vec()))
    })
}

/// Deletes one kind of secret for an account. Blocking.
#[allow(dead_code)] // Used by the IMAP/POP3 account setup (milestone 2).
pub fn delete_secret(account: &AccountId, kind: &str) -> Result<(), VaultError> {
    let attrs = attributes(account, kind);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        keyring.delete(&borrow(&attrs)).await.map_err(|e| VaultError::Unavailable(e.to_string()))
    })
}

/// Deletes every secret stored for an account, whatever its kind. Blocking.
pub fn delete_all_secrets(account: &AccountId) -> Result<(), VaultError> {
    let attrs = account_attributes(account);
    runtime().block_on(async move {
        let keyring = oo7::Keyring::new().await.map_err(|e| VaultError::Unavailable(e.to_string()))?;
        keyring.delete(&borrow(&attrs)).await.map_err(|e| VaultError::Unavailable(e.to_string()))
    })
}

// ------------------------------------------------------------------ typed wrappers

/// Saves OAuth tokens for a Gmail account, replacing any previous entry. Blocking.
pub fn save_tokens(account: &AccountId, tokens: &OAuthTokens) -> Result<(), VaultError> {
    let secret = serde_json::to_vec(tokens).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    save_secret(account, KIND_OAUTH_TOKENS, secret)
}

/// Loads OAuth tokens for a Gmail account. `Ok(None)` when nothing is stored. Blocking.
pub fn load_tokens(account: &AccountId) -> Result<Option<OAuthTokens>, VaultError> {
    match load_secret(account, KIND_OAUTH_TOKENS)? {
        Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| VaultError::Corrupt(e.to_string())),
        None => Ok(None),
    }
}

/// Deletes stored OAuth tokens for an account. Blocking.
#[allow(dead_code)] // Kept for callers that only want to drop the Gmail sign-in; remove_account uses delete_all_secrets.
pub fn delete_tokens(account: &AccountId) -> Result<(), VaultError> {
    delete_secret(account, KIND_OAUTH_TOKENS)
}

/// Saves a server password (`KIND_PASSWORD_INCOMING` or `KIND_PASSWORD_OUTGOING`). Blocking.
#[allow(dead_code)] // Used by the IMAP/POP3 account setup (milestone 2).
pub fn save_password(account: &AccountId, kind: &str, password: &str) -> Result<(), VaultError> {
    save_secret(account, kind, password.as_bytes().to_vec())
}

/// Loads a server password. `Ok(None)` when nothing is stored. Blocking.
#[allow(dead_code)] // Used by providers_for once IMAP/POP3 providers exist (milestone 2).
pub fn load_password(account: &AccountId, kind: &str) -> Result<Option<String>, VaultError> {
    match load_secret(account, kind)? {
        Some(bytes) => String::from_utf8(bytes).map(Some).map_err(|e| VaultError::Corrupt(e.to_string())),
        None => Ok(None),
    }
}
