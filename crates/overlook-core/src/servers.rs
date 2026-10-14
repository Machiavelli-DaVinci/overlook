//! Server settings for IMAP, POP3 and SMTP accounts.
//!
//! These are not secrets and live in the store next to the account. Passwords never appear
//! here: they are kept in the system keyring under the account id, so a `ServerEndpoint`
//! can be logged or shown in the settings dialog without leaking anything.

use serde::{Deserialize, Serialize};

/// How the connection is protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    /// Implicit TLS from the first byte (IMAPS 993, POP3S 995, SMTPS 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS / STLS (IMAP 143, POP3 110, SMTP 587).
    StartTls,
    /// No encryption. Only acceptable on localhost, for tests and local bridges.
    None,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Security::Tls => "SSL/TLS",
            Security::StartTls => "STARTTLS",
            Security::None => "None",
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::None => "none",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "tls" | "ssl" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "none" | "plain" => Some(Security::None),
            _ => None,
        }
    }
}

/// How the client proves who it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    /// `LOGIN` / `AUTH PLAIN` with the password from the keyring.
    #[default]
    Password,
    /// XOAUTH2. Reserved in the model; no provider implements it yet.
    OAuth2,
}

/// The protocol an endpoint speaks. Determines the default port for a security mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerProtocol {
    Imap,
    Pop3,
    Smtp,
}

impl ServerProtocol {
    /// The conventional port for this protocol and security mode.
    pub fn default_port(self, security: Security) -> u16 {
        match (self, security) {
            (ServerProtocol::Imap, Security::Tls) => 993,
            (ServerProtocol::Imap, _) => 143,
            (ServerProtocol::Pop3, Security::Tls) => 995,
            (ServerProtocol::Pop3, _) => 110,
            (ServerProtocol::Smtp, Security::Tls) => 465,
            (ServerProtocol::Smtp, Security::StartTls) => 587,
            (ServerProtocol::Smtp, Security::None) => 25,
        }
    }
}

/// One server to connect to. The password is not part of this type on purpose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEndpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    /// The login name; usually the email address.
    pub username: String,
    #[serde(default)]
    pub auth: AuthMethod,
}

impl ServerEndpoint {
    pub fn new(host: impl Into<String>, port: u16, security: Security, username: impl Into<String>) -> Self {
        Self { host: host.into(), port, security, username: username.into(), auth: AuthMethod::Password }
    }

    /// `host:port` for connecting and for messages.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Whether unencrypted connections are being asked for somewhere other than this machine.
    pub fn is_insecure_remote(&self) -> bool {
        self.security == Security::None && !is_local_host(&self.host)
    }
}

fn is_local_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1" || h.ends_with(".localhost")
}

/// What happens to mail on a POP3 server after it has been downloaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pop3Policy {
    /// Keep messages on the server after downloading them.
    pub leave_on_server: bool,
    /// With `leave_on_server`, delete server copies this many days after they were first seen.
    pub delete_after_days: Option<u32>,
    /// Delete the server copy when the local message is moved to Trash.
    pub delete_on_server_when_trashed: bool,
}

impl Default for Pop3Policy {
    fn default() -> Self {
        Self { leave_on_server: true, delete_after_days: None, delete_on_server_when_trashed: false }
    }
}

/// Per-account IMAP folder choices that override SPECIAL-USE and name heuristics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImapPolicy {
    /// Folder path that the Archive operation moves mail to.
    pub archive_folder: Option<String>,
    /// Folder path that holds sent mail, when the server does not advertise one.
    pub sent_folder: Option<String>,
    /// Folder path that holds trashed mail, when the server does not advertise one.
    pub trash_folder: Option<String>,
    /// Without a Trash folder, flag `\Deleted` and expunge instead of moving.
    pub expunge_on_trash: bool,
}

/// Everything needed to reach an IMAP or POP3 account and its SMTP server, except passwords.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailServers {
    /// IMAP or POP3, depending on the account's provider kind.
    pub incoming: ServerEndpoint,
    /// SMTP.
    pub outgoing: ServerEndpoint,
    #[serde(default)]
    pub pop3: Option<Pop3Policy>,
    #[serde(default)]
    pub imap: Option<ImapPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json_with_defaults() {
        let servers = MailServers {
            incoming: ServerEndpoint::new("imap.example.com", 993, Security::Tls, "jason@example.com"),
            outgoing: ServerEndpoint::new("smtp.example.com", 587, Security::StartTls, "jason@example.com"),
            pop3: None,
            imap: Some(ImapPolicy { archive_folder: Some("Archive".into()), ..Default::default() }),
        };
        let json = serde_json::to_string(&servers).unwrap();
        assert!(json.contains("\"security\":\"start-tls\""), "{json}");
        assert_eq!(serde_json::from_str::<MailServers>(&json).unwrap(), servers);

        // Older rows without the optional fields still load.
        let minimal = r#"{"incoming":{"host":"pop.example.com","port":995,"username":"j"},"outgoing":{"host":"smtp.example.com","port":465,"username":"j"}}"#;
        let loaded: MailServers = serde_json::from_str(minimal).unwrap();
        assert_eq!(loaded.incoming.security, Security::Tls);
        assert_eq!(loaded.incoming.auth, AuthMethod::Password);
        assert!(loaded.pop3.is_none() && loaded.imap.is_none());
        assert_eq!(Pop3Policy::default().leave_on_server, true);
    }

    #[test]
    fn default_ports_and_local_hosts() {
        assert_eq!(ServerProtocol::Imap.default_port(Security::Tls), 993);
        assert_eq!(ServerProtocol::Smtp.default_port(Security::StartTls), 587);
        assert_eq!(ServerProtocol::Pop3.default_port(Security::None), 110);
        assert!(!ServerEndpoint::new("localhost", 1025, Security::None, "x").is_insecure_remote());
        assert!(ServerEndpoint::new("mail.example.com", 25, Security::None, "x").is_insecure_remote());
        assert_eq!(Security::from_key("ssl"), Some(Security::Tls));
    }
}
