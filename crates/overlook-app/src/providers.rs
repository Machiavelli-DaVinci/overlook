//! Builds the providers for each configured account.

use crate::credentials;
use overlook_core::*;
use overlook_provider::gmail::{GmailClient, GmailProvider, GoogleCalendarProvider, OAuthTokens};
use overlook_provider::imap::ImapProvider;
use overlook_provider::pop3::Pop3Provider;
use overlook_provider::sample::SampleProvider;
use overlook_provider::{CalendarProvider, MailProvider};
use overlook_store::Store;
use std::sync::Arc;
use tracing::warn;

/// What the Status row asks for when an IMAP or POP3 account has no usable password.
pub const PASSWORD_NEEDED_WORDING: &str = "Enter this account’s password again to continue syncing. Your local drafts are safe.";

/// Why an account has no usable provider right now.
#[derive(Debug, Clone)]
pub enum ProviderIssue {
    /// No tokens are stored, or the keyring could not be read.
    SignInRequired(String),
}

#[derive(Clone)]
pub struct AccountProviders {
    pub mail: Arc<dyn MailProvider>,
    pub calendar: Option<Arc<dyn CalendarProvider>>,
}

pub fn gmail_providers(account: &Account, tokens: OAuthTokens) -> Result<AccountProviders, String> {
    let config = crate::google_signin::load_client_config()?;
    let account_id = account.id.clone();
    let client = Arc::new(GmailClient::new(
        config,
        tokens,
        Box::new(move |fresh| {
            if let Err(e) = credentials::save_tokens(&account_id, fresh) {
                warn!("could not persist refreshed tokens: {e}");
            }
        }),
    ));
    let inbox_limit = crate::store_prefs_inbox_limit();
    Ok(AccountProviders {
        mail: Arc::new(GmailProvider::new(client.clone()).with_inbox_limit(inbox_limit)),
        calendar: Some(Arc::new(GoogleCalendarProvider::new(client))),
    })
}

/// An IMAP account: one connection for mail, SMTP for sending, no calendar. The outgoing
/// password is the incoming one unless a separate one was stored at setup.
pub fn imap_providers(servers: MailServers, password: String, outgoing_password: Option<String>) -> AccountProviders {
    let inbox_limit = crate::store_prefs_inbox_limit();
    AccountProviders { mail: Arc::new(ImapProvider::new(servers, password).with_outgoing_password(outgoing_password).with_inbox_limit(inbox_limit)), calendar: None }
}

/// Where a POP3 account's downloaded messages live: `<data dir>/raw/<account>/`, one `.eml`
/// per message. This is the account's primary copy; "Back up mail" includes it.
pub fn raw_dir_for(account: &AccountId) -> std::path::PathBuf {
    crate::data_dir().join("raw").join(account.as_str().replace([':', '/'], "_"))
}

/// A POP3 account: one session per sync, SMTP for sending, no calendar. The local store
/// and `raw/` hold the only copy of its mail.
pub fn pop3_providers(account: &AccountId, servers: MailServers, password: String, outgoing_password: Option<String>) -> AccountProviders {
    AccountProviders { mail: Arc::new(Pop3Provider::new(servers, password, raw_dir_for(account)).with_outgoing_password(outgoing_password)), calendar: None }
}

/// The stored passwords of an IMAP or POP3 account: the incoming one, and the outgoing one
/// when a separate one was saved.
fn stored_passwords(account: &Account) -> Result<(String, Option<String>), ProviderIssue> {
    match credentials::load_password(&account.id, credentials::KIND_PASSWORD_INCOMING) {
        Ok(Some(password)) => {
            let outgoing = match credentials::load_password(&account.id, credentials::KIND_PASSWORD_OUTGOING) {
                Ok(found) => found,
                Err(e) => {
                    warn!("could not read the outgoing password for {}: {e}; using the incoming one", account.email);
                    None
                }
            };
            Ok((password, outgoing))
        }
        Ok(None) => Err(ProviderIssue::SignInRequired(PASSWORD_NEEDED_WORDING.into())),
        Err(e) => Err(ProviderIssue::SignInRequired(e.to_string())),
    }
}

/// Builds the providers for `account`, reading stored tokens or passwords where needed.
pub fn providers_for(account: &Account, sample: &SampleProvider, store: &Store) -> Result<AccountProviders, ProviderIssue> {
    match account.provider {
        ProviderKind::Sample => Ok(AccountProviders { mail: Arc::new(sample.clone()), calendar: Some(Arc::new(sample.clone())) }),
        ProviderKind::Gmail => match credentials::load_tokens(&account.id) {
            Ok(Some(tokens)) => gmail_providers(account, tokens).map_err(ProviderIssue::SignInRequired),
            Ok(None) => Err(ProviderIssue::SignInRequired("Sign in again to continue syncing. Your local drafts are safe.".into())),
            Err(e) => Err(ProviderIssue::SignInRequired(e.to_string())),
        },
        ProviderKind::Imap | ProviderKind::Pop3 => {
            let servers = match store.servers(&account.id) {
                Ok(Some(s)) => s,
                Ok(None) => return Err(ProviderIssue::SignInRequired("Finish setting up this account’s servers to start syncing.".into())),
                Err(e) => return Err(ProviderIssue::SignInRequired(e.to_string())),
            };
            let (password, outgoing) = stored_passwords(account)?;
            Ok(if account.provider == ProviderKind::Imap { imap_providers(servers, password, outgoing) } else { pop3_providers(&account.id, servers, password, outgoing) })
        }
    }
}
