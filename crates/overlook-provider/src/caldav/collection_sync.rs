//! RFC 6578 is an optimization, never authority to discard a partial download.
use super::*;
use roxmltree::{Document, ParsingOptions};

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl CalDavClient {
    /// Only an empty, complete multistatus with one valid token proves that all
    /// cached projections can be retained. Changes, truncation and expired tokens
    /// fall back to a full bounded collection query, without advancing the cursor.
    pub(super) fn unchanged_collection(
        &self,
        url: &Url,
        token: &str,
    ) -> Result<Option<String>, CalDavError> {
        let body = format!(
            r#"<d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
            escape(token)
        );
        let (_, body) = match self.request_xml(
            url,
            "REPORT",
            "1",
            &body,
            MAX_RESPONSE_BYTES,
            Instant::now(),
        ) {
            Ok(result) => result,
            Err(
                CalDavError::Forbidden
                | CalDavError::InvalidResponse(_)
                | CalDavError::UnexpectedStatus(400 | 405 | 409 | 410 | 415 | 501 | 507),
            ) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(unchanged_token(&body))
    }
}

fn unchanged_token(body: &str) -> Option<String> {
    let doc = Document::parse_with_options(
        body,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 50_000,
            ..Default::default()
        },
    )
    .ok()?;
    let root = doc.root_element();
    if !root.has_tag_name(("DAV:", "multistatus")) {
        return None;
    }
    let children: Vec<_> = root.children().filter(|n| n.is_element()).collect();
    if children.len() != 1
        || !children[0].has_tag_name(("DAV:", "sync-token"))
        || children[0].children().any(|n| n.is_element())
    {
        return None;
    }
    let token: String = children[0]
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    if token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
        return None;
    }
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_empty_sync_reports_allow_reusing_events() {
        let wrap =
            |inner: &str| format!(r#"<d:multistatus xmlns:d="DAV:">{inner}</d:multistatus>"#);
        assert_eq!(
            unchanged_token(&wrap(
                "<d:sync-token>https://server/sync/2&amp;x</d:sync-token>"
            )),
            Some("https://server/sync/2&x".into())
        );
        for inner in ["", "<d:sync-token/>", "<d:sync-token>a</d:sync-token><d:sync-token>b</d:sync-token>",
            "<d:response><d:status>HTTP/1.1 404 Gone</d:status></d:response><d:sync-token>new</d:sync-token>",
            "<d:response><d:status>HTTP/1.1 507 Incomplete</d:status></d:response><d:sync-token>new</d:sync-token>",
            "<d:error/><d:sync-token>new</d:sync-token>", "<sync-token>wrong namespace</sync-token>",
            "<d:sync-token><d:error/></d:sync-token>"] {
            assert!(unchanged_token(&wrap(inner)).is_none(), "{inner}");
        }
        assert!(unchanged_token("<!DOCTYPE x [<!ENTITY x 'expand'>]><x>&x;</x>").is_none());
    }
}
