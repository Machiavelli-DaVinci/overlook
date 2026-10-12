//! Builds the providers for each configured account.

use crate::credentials;
use overlook_core::*;
use overlook_provider::gmail::{GmailClient, GmailProvider, GoogleCalendarProvider, OAuthTokens};
use overlook_provider::sample::SampleProvider;
use overlook_provider::{CalendarProvider, MailProvider};
use std::sync::Arc;
use tracing::warn;

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

/// Builds the providers for `account`, reading stored tokens where needed.
pub fn providers_for(account: &Account, sample: &SampleProvider) -> Result<AccountProviders, ProviderIssue> {
    match account.provider {
        ProviderKind::Sample => Ok(AccountProviders { mail: Arc::new(sample.clone()), calendar: Some(Arc::new(sample.clone())) }),
        ProviderKind::Gmail => match credentials::load_tokens(&account.id) {
            Ok(Some(tokens)) => gmail_providers(account, tokens).map_err(ProviderIssue::SignInRequired),
            Ok(None) => Err(ProviderIssue::SignInRequired("Sign in again to continue syncing. Your local drafts are safe.".into())),
            Err(e) => Err(ProviderIssue::SignInRequired(e.to_string())),
        },
        ProviderKind::Imap | ProviderKind::Pop3 => Err(ProviderIssue::SignInRequired("This account type is not supported yet.".into())),
    }
}
