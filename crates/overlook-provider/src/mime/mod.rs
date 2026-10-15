//! Shared MIME handling for providers that exchange raw RFC 822 bytes (IMAP, POP3, SMTP).
//!
//! - `parse`: raw message to [`overlook_core::Message`] through `mail-parser`.
//! - [`decode_rfc2047`]: encoded-word decoding for header values that arrive undecoded
//!   (Gmail JSON headers, IMAP `ENVELOPE` subjects).

pub mod parse;

pub use parse::{headers_from_rfc822, message_from_rfc822, part_bytes, MessageFlags};

use base64::Engine;

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

    #[test]
    fn decodes_encoded_words() {
        assert_eq!(decode_rfc2047("=?UTF-8?B?SMOpbGxv?= world"), "Héllo world");
        assert_eq!(decode_rfc2047("=?utf-8?Q?Caf=C3=A9_time?="), "Café time");
        assert_eq!(decode_rfc2047("plain"), "plain");
        assert_eq!(decode_rfc2047("=?UTF-8?B?YQ==?= =?UTF-8?B?Yg==?="), "ab");
        assert_eq!(decode_rfc2047("=?ISO-8859-1?Q?Caf=E9?="), "Café");
    }
}
