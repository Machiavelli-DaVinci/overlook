//! Gmail JSON to Overlook domain types.
//!
//! Local ids are deterministic (`<account>:<gmail id>`) so repeated downloads update the
//! same rows and conversations map one to one onto Gmail threads.

use base64::Engine;
use overlook_core::sanitize::html_to_text;
use overlook_core::*;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

pub fn local_message_id(account: &AccountId, gmail_id: &str) -> MessageId {
    MessageId(format!("{}:{gmail_id}", account.0))
}

pub fn local_conversation_id(account: &AccountId, thread_id: &str) -> ConversationId {
    ConversationId(format!("{}:{thread_id}", account.0))
}

pub fn local_label_id(account: &AccountId, gmail_label_id: &str) -> LabelId {
    LabelId(format!("{}:{gmail_label_id}", account.0))
}

/// Strips the account prefix from a local id to recover the Gmail id.
pub fn gmail_id(account: &AccountId, local: &str) -> Option<String> {
    local.strip_prefix(&format!("{}:", account.0)).map(str::to_owned)
}

/// Maps a Gmail label. Returns `None` for Gmail-internal labels Overlook models as flags
/// or views (UNREAD, STARRED, DRAFT, IMPORTANT, CHAT, categories).
pub fn label_from_json(account: &AccountId, v: &Value, sort_order: i32) -> Option<Label> {
    let id = v.get("id")?.as_str()?;
    let name = v.get("name")?.as_str()?;
    let system = v.get("type").and_then(|t| t.as_str()) == Some("system");
    let kind = if system {
        match id {
            "INBOX" => LabelKind::Inbox,
            "SENT" => LabelKind::Sent,
            "TRASH" => LabelKind::Trash,
            "SPAM" => LabelKind::Spam,
            _ => return None,
        }
    } else {
        LabelKind::User
    };
    let display = match kind {
        LabelKind::Inbox => "Inbox",
        LabelKind::Sent => "Sent",
        LabelKind::Trash => "Trash",
        LabelKind::Spam => "Spam",
        _ => name,
    };
    let color = v
        .get("color")
        .and_then(|c| c.get("backgroundColor"))
        .and_then(|c| c.as_str())
        .map(color_token);
    Some(Label {
        id: local_label_id(account, id),
        account_id: account.clone(),
        kind,
        name: display.to_owned(),
        provider_id: Some(ProviderId(id.to_owned())),
        color,
        sort_order: if system { sort_order } else { 100 + sort_order },
    })
}

/// Reduces a Gmail label color to one of Overlook's label dot tokens.
fn color_token(hex: &str) -> String {
    let rgb = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    if r > g && r > b && r > 150 {
        "amber".into()
    } else if b > r && b >= g {
        if r > 120 { "plum".into() } else { "slate".into() }
    } else {
        "slate".into()
    }
}

/// Maps a `users.messages.get` response (format `full`, `metadata`, or `minimal`).
pub fn message_from_json(account: &Account, v: &Value, labels: &HashMap<String, LabelId>) -> Option<Message> {
    let gmail_id = v.get("id")?.as_str()?;
    let thread_id = v.get("threadId").and_then(|t| t.as_str()).unwrap_or(gmail_id);
    let label_ids_raw: Vec<String> = v
        .get("labelIds")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let has = |name: &str| label_ids_raw.iter().any(|l| l == name);

    let payload = v.get("payload");
    let headers = payload.and_then(|p| p.get("headers")).and_then(|h| h.as_array());
    let header = |name: &str| -> Option<String> {
        headers?
            .iter()
            .find(|h| h.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .and_then(|h| h.get("value"))
            .and_then(|x| x.as_str())
            .map(decode_rfc2047)
    };

    let from = header("From").map(|f| Address::parse_list(&f).into_iter().next().unwrap_or_default()).unwrap_or_default();
    let to = header("To").map(|f| Address::parse_list(&f)).unwrap_or_default();
    let cc = header("Cc").map(|f| Address::parse_list(&f)).unwrap_or_default();
    let bcc = header("Bcc").map(|f| Address::parse_list(&f)).unwrap_or_default();
    let reply_to = header("Reply-To").and_then(|f| Address::parse_list(&f).into_iter().next());
    let subject = header("Subject").unwrap_or_default();
    let date = v
        .get("internalDate")
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<i64>().ok())
        .unwrap_or(0);
    let snippet = v.get("snippet").and_then(|s| s.as_str()).map(html_to_text).unwrap_or_default();

    let mut text = None;
    let mut html = None;
    let mut attachments = Vec::new();
    let local_id = local_message_id(&account.id, gmail_id);
    if let Some(p) = payload {
        collect_parts(p, &mut text, &mut html, &mut attachments, &local_id);
    }
    if text.is_none() && html.is_some() {
        text = html.as_deref().map(html_to_text);
    }

    let mut label_ids = BTreeSet::new();
    for raw in &label_ids_raw {
        if let Some(local) = labels.get(raw) {
            label_ids.insert(local.clone());
        }
    }
    let outgoing = has("SENT") || from.email.eq_ignore_ascii_case(&account.email);

    Some(Message {
        id: local_id,
        account_id: account.id.clone(),
        conversation_id: local_conversation_id(&account.id, thread_id),
        provider_id: Some(ProviderId(gmail_id.to_owned())),
        from,
        to,
        cc,
        bcc,
        reply_to,
        subject,
        snippet,
        body: Body { text, html },
        date,
        is_read: !has("UNREAD"),
        is_starred: has("STARRED"),
        is_outgoing: outgoing,
        label_ids,
        attachments,
        rfc_message_id: header("Message-ID").or_else(|| header("Message-Id")),
        in_reply_to: header("In-Reply-To"),
        category: label_ids_raw.iter().find_map(|l| l.strip_prefix("CATEGORY_").map(|c| c.to_ascii_lowercase())),
        is_important: has("IMPORTANT"),
        focus_override: None,
    })
}

fn collect_parts(part: &Value, text: &mut Option<String>, html: &mut Option<String>, attachments: &mut Vec<Attachment>, message_id: &MessageId) {
    let mime = part.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
    let filename = part.get("filename").and_then(|f| f.as_str()).unwrap_or("");
    let body = part.get("body");
    if !filename.is_empty() {
        let size = body.and_then(|b| b.get("size")).and_then(|s| s.as_u64()).unwrap_or(0);
        let attachment_id = body.and_then(|b| b.get("attachmentId")).and_then(|a| a.as_str());
        attachments.push(Attachment {
            id: AttachmentId(format!("{}:att-{}", message_id.0, attachments.len())),
            message_id: message_id.clone(),
            filename: filename.to_owned(),
            mime_type: mime.to_owned(),
            size_bytes: size,
            local_path: None,
            provider_id: attachment_id.map(|a| ProviderId(a.to_owned())),
        });
    } else if let Some(data) = body.and_then(|b| b.get("data")).and_then(|d| d.as_str()) {
        let decoded = String::from_utf8_lossy(&base64url_decode(data)).into_owned();
        match mime {
            "text/plain" if text.is_none() => *text = Some(decoded),
            "text/html" if html.is_none() => *html = Some(decoded),
            _ => {}
        }
    }
    if let Some(parts) = part.get("parts").and_then(|p| p.as_array()) {
        for child in parts {
            collect_parts(child, text, html, attachments, message_id);
        }
    }
}

pub fn base64url_decode(data: &str) -> Vec<u8> {
    let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cleaned.trim_end_matches('='))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&cleaned))
        .unwrap_or_default()
}

pub fn base64url_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Decodes RFC 2047 encoded words (`=?UTF-8?B?...?=` / `=?UTF-8?Q?...?=`) in a header.
pub fn decode_rfc2047(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut last_was_word = false;
    while let Some(start) = rest.find("=?") {
        let before = &rest[..start];
        if !(last_was_word && before.trim().is_empty()) {
            out.push_str(before);
        }
        let Some(end_rel) = rest[start + 2..].find("?=") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let word = &rest[start + 2..start + 2 + end_rel];
        let mut pieces = word.splitn(3, '?');
        let (charset, encoding, payload) = (pieces.next().unwrap_or(""), pieces.next().unwrap_or(""), pieces.next().unwrap_or(""));
        let bytes = match encoding.to_ascii_uppercase().as_str() {
            "B" => base64::engine::general_purpose::STANDARD.decode(payload).unwrap_or_default(),
            "Q" => decode_q(payload),
            _ => payload.as_bytes().to_vec(),
        };
        let decoded = if charset.to_ascii_lowercase().starts_with("iso-8859-1") || charset.to_ascii_lowercase() == "latin1" {
            bytes.iter().map(|&b| b as char).collect()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        out.push_str(&decoded);
        rest = &rest[start + 2 + end_rel + 2..];
        last_was_word = true;
    }
    out.push_str(rest);
    out
}

fn decode_q(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => {
                out.push(b' ');
                i += 1;
            }
            b'=' if i + 2 < bytes.len() + 1 && i + 2 <= bytes.len() - 1 + 1 && i + 2 < bytes.len() + 1 => {
                if i + 2 < bytes.len() + 1 && i + 3 <= bytes.len() {
                    if let Ok(v) = u8::from_str_radix(&payload[i + 1..i + 3], 16) {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                }
                out.push(b'=');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: AccountId::from("g1"),
            provider: ProviderKind::Gmail,
            display_name: "Jason".into(),
            email: "jason@example.com".into(),
            created_at: 0,
        }
    }

    #[test]
    fn decodes_encoded_words() {
        assert_eq!(decode_rfc2047("=?UTF-8?B?SMOpbGxv?= world"), "Héllo world");
        assert_eq!(decode_rfc2047("=?utf-8?Q?Caf=C3=A9_time?="), "Café time");
        assert_eq!(decode_rfc2047("plain"), "plain");
        assert_eq!(decode_rfc2047("=?UTF-8?B?YQ==?= =?UTF-8?B?Yg==?="), "ab");
    }

    #[test]
    fn maps_labels() {
        let acct = AccountId::from("g1");
        let inbox = label_from_json(&acct, &serde_json::json!({"id":"INBOX","name":"INBOX","type":"system"}), 0).unwrap();
        assert_eq!(inbox.kind, LabelKind::Inbox);
        assert_eq!(inbox.name, "Inbox");
        assert_eq!(inbox.provider_id.as_ref().unwrap().as_str(), "INBOX");
        assert!(label_from_json(&acct, &serde_json::json!({"id":"UNREAD","name":"UNREAD","type":"system"}), 0).is_none());
        let user = label_from_json(&acct, &serde_json::json!({"id":"Label_7","name":"Projects","type":"user","color":{"backgroundColor":"#fb4c2f"}}), 3).unwrap();
        assert_eq!(user.kind, LabelKind::User);
        assert_eq!(user.color.as_deref(), Some("amber"));
    }

    #[test]
    fn maps_full_message() {
        let json = serde_json::json!({
            "id": "18f1", "threadId": "18f0", "labelIds": ["INBOX", "UNREAD", "STARRED", "Label_7", "IMPORTANT"],
            "snippet": "Hi Jason &amp; team",
            "internalDate": "1760000000000",
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [
                    {"name": "From", "value": "=?UTF-8?B?TWF5YSBDaGVu?= <maya@northstar.example>"},
                    {"name": "To", "value": "Jason <jason@example.com>, alex@studio.example"},
                    {"name": "Subject", "value": "Launch creative"},
                    {"name": "Message-ID", "value": "<abc@northstar.example>"}
                ],
                "parts": [
                    {"mimeType": "multipart/alternative", "parts": [
                        {"mimeType": "text/plain", "body": {"data": base64url_encode(b"Hello there")}},
                        {"mimeType": "text/html", "body": {"data": base64url_encode(b"<p>Hello there</p>")}}
                    ]},
                    {"mimeType": "application/pdf", "filename": "brief.pdf", "body": {"attachmentId": "ATT1", "size": 1234}}
                ]
            }
        });
        let labels: HashMap<String, LabelId> = [
            ("INBOX".to_owned(), LabelId::from("g1:INBOX")),
            ("Label_7".to_owned(), LabelId::from("g1:Label_7")),
        ]
        .into_iter()
        .collect();
        let m = message_from_json(&account(), &json, &labels).unwrap();
        assert_eq!(m.id.as_str(), "g1:18f1");
        assert_eq!(m.conversation_id.as_str(), "g1:18f0");
        assert_eq!(m.from.name.as_deref(), Some("Maya Chen"));
        assert_eq!(m.to.len(), 2);
        assert_eq!(m.subject, "Launch creative");
        assert_eq!(m.snippet, "Hi Jason & team");
        assert!(!m.is_read);
        assert!(m.is_starred);
        assert_eq!(m.label_ids.len(), 2, "UNREAD and STARRED are flags, not labels");
        assert_eq!(m.category, None);
        assert_eq!(m.body.text.as_deref(), Some("Hello there"));
        assert_eq!(m.body.html.as_deref(), Some("<p>Hello there</p>"));
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].provider_id.as_ref().unwrap().as_str(), "ATT1");
        assert_eq!(m.rfc_message_id.as_deref(), Some("<abc@northstar.example>"));
        assert!(m.is_important);
        assert_eq!(gmail_id(&account().id, m.id.as_str()).as_deref(), Some("18f1"));
        let promo = serde_json::json!({"id": "p1", "threadId": "p1", "labelIds": ["INBOX", "CATEGORY_PROMOTIONS"]});
        assert_eq!(message_from_json(&account(), &promo, &HashMap::new()).unwrap().category.as_deref(), Some("promotions"));
    }

    #[test]
    fn maps_minimal_message_without_payload() {
        let json = serde_json::json!({"id": "x1", "threadId": "t1", "labelIds": ["SENT"]});
        let m = message_from_json(&account(), &json, &HashMap::new()).unwrap();
        assert!(m.is_outgoing);
        assert!(m.is_read);
        assert!(!m.body.is_cached());
    }
}
