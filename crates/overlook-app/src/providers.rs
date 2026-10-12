//! Builds the provider for each configured account.

use crate::credentials;
use overlook_core::*;
use overlook_provider::gmail::{GmailClient, GmailProvider, OAuthTokens};
use overlook_provider::sample::SampleProvider;
use overlook_provider::MailProvider;
use std::sync::Arc;
use tracing::warn;

/// Why an account has no usable provider right now.
#[derive(Debug, Clone)]
pub enum ProviderIssue {
    /// No tokens are stored, or the keyring could not be read.
    SignInRequired(String),
}

pub fn gmail_provider(account: &Account, tokens: OAuthTokens) -> Result<Arc<dyn MailProvider>, String> {
    let config = crate::google_signin::load_client_config()?;
    let account_id = account.id.clone();
    let client = GmailClient::new(
        config,
        tokens,
        Box::new(move |fresh| {
            if let Err(e) = credentials::save_tokens(&account_id, fresh) {
                warn!("could not persist refreshed tokens: {e}");
            }
        }),
    );
    Ok(Arc::new(GmailProvider::new(Arc::new(client))))
}

/// Builds the provider for `account`, reading stored tokens where needed.
pub fn provider_for(account: &Account, sample: &SampleProvider) -> Result<Arc<dyn MailProvider>, ProviderIssue> {
    match account.provider {
        ProviderKind::Sample => Ok(Arc::new(sample.clone())),
        ProviderKind::Gmail => match credentials::load_tokens(&account.id) {
            Ok(Some(tokens)) => gmail_provider(account, tokens).map_err(ProviderIssue::SignInRequired),
            Ok(None) => Err(ProviderIssue::SignInRequired("Sign in again to continue syncing. Your local drafts are safe.".into())),
            Err(e) => Err(ProviderIssue::SignInRequired(e.to_string())),
        },
        ProviderKind::Imap | ProviderKind::Pop3 => Err(ProviderIssue::SignInRequired("This account type is not supported yet.".into())),
    }
}
