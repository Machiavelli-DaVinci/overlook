//! Gmail through the Gmail API.
//!
//! - `oauth`: installed-app OAuth 2.0 (PKCE, loopback redirect, token exchange/refresh).
//! - `api`: a small authenticated REST client with transparent token refresh.
//! - `mapping`: Gmail JSON to Overlook's domain types.
//! - `mime`: building RFC 822 messages for `messages.send`.
//! - `provider`: the `MailProvider` implementation.

pub mod api;
pub mod mapping;
pub mod mime;
pub mod oauth;
pub mod provider;

pub use api::GmailClient;
pub use oauth::{AuthorizationRequest, ClientConfig, OAuthTokens, Pkce, UserInfo};
pub use provider::GmailProvider;
