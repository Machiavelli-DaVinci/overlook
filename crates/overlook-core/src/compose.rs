//! Outgoing message bodies.
//!
//! The composer edits Markdown-flavoured text: `**bold**`, `*italic*`, `~~strike~~`,
//! `` `code` ``, `[text](url)`, `- ` and `1. ` lists, `> ` quotes, `# ` headings, bare
//! URLs. [`render_markdown`] turns it into the HTML part of the message and
//! [`markdown_to_plain`] into the text part. [`outgoing_bodies`] appends the quoted original
//! of a reply or forward to both.

use crate::model::{forward_header, quote_header, quote_text, Draft, DraftKind, Message};
use crate::sanitize::{clean_only, escape_html, html_to_text, text_to_html, RemoteContent};

/// Renders the composer's text as HTML. Raw HTML typed by the user is escaped, not passed
/// through, so a message can never carry markup the author did not see.
pub fn render_markdown(src: &str) -> String {
    let mut options = markdown::Options::gfm();
    options.compile.allow_dangerous_html = false;
    options.compile.allow_dangerous_protocol = false;
    // A single newline inside a paragraph is a line break, the way mail is typed.
    options.parse.constructs.hard_break_trailing = true;
    let html = markdown::to_html_with_options(src, &options).unwrap_or_else(|_| text_to_html(src));
    soft_breaks_to_br(&html)
}

/// The markdown crate keeps a soft line break as a newline inside the paragraph; mail
/// readers collapse that to a space, so it becomes `<br>`.
fn soft_breaks_to_br(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_pre = false;
    let mut in_tag = false;
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '<' {
            in_tag = true;
            if html[i..].starts_with("<pre") {
                in_pre = true;
            } else if html[i..].starts_with("</pre") {
                in_pre = false;
            }
        } else if c == '>' {
            in_tag = false;
        }
        if c == '\n' && !in_tag && !in_pre {
            // A newline right after a block close or before a block open is layout, not a break.
            let before = html[..i].trim_end_matches(' ');
            let after = html[i + 1..].trim_start_matches(' ');
            if before.ends_with('>') || after.starts_with('<') {
                out.push('\n');
            } else {
                out.push_str("<br>\n");
            }
        } else {
            out.push(c);
        }
        i += c.len_utf8();
    }
    out
}

/// The text part: the author's words with the formatting marks removed, links kept readable.
pub fn markdown_to_plain(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for (i, line) in src.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        // Headings lose their hashes; everything else keeps its prefix (lists, quotes).
        let body = if let Some(rest) = trimmed.strip_prefix('#') {
            rest.trim_start_matches('#').trim_start()
        } else {
            trimmed
        };
        out.push_str(indent);
        out.push_str(&strip_inline(body));
    }
    out
}

fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut open_star = false;
    let mut open_under = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**").or_else(|| rest.strip_prefix("__")).or_else(|| rest.strip_prefix("~~")) {
            rest = after;
            continue;
        }
        if rest.starts_with('`') {
            rest = &rest[1..];
            continue;
        }
        if rest.starts_with('[') {
            if let Some((text, url, len)) = parse_link(rest) {
                out.push_str(text);
                if !url.is_empty() && url != text {
                    out.push_str(" (");
                    out.push_str(url);
                    out.push(')');
                }
                rest = &rest[len..];
                continue;
            }
        }
        // Single `*` or `_` emphasis: a marker closes an open run, or opens one when a closer
        // follows on the line. `_` only counts at a word boundary, so snake_case stays intact.
        if rest.starts_with('*') || rest.starts_with('_') {
            let marker = rest.as_bytes()[0];
            let open = if marker == b'*' { &mut open_star } else { &mut open_under };
            let at_word_boundary = marker == b'*' || !out.chars().next_back().is_some_and(|c| c.is_alphanumeric());
            if *open {
                *open = false;
                rest = &rest[1..];
                continue;
            }
            if at_word_boundary && emphasis_closes(rest) {
                *open = true;
                rest = &rest[1..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn emphasis_closes(rest: &str) -> bool {
    let marker = rest.as_bytes()[0];
    let inner = &rest[1..];
    if inner.starts_with(' ') {
        return false;
    }
    match inner.find(marker as char) {
        Some(0) | None => false,
        Some(i) => marker == b'*' || !inner[i + 1..].starts_with(|c: char| c.is_alphanumeric()),
    }
}

/// `[text](url)` at the start of `s`: (text, url, consumed length).
pub fn parse_link(s: &str) -> Option<(&str, &str, usize)> {
    let close = s.find("](")?;
    let text = &s[1..close];
    if text.contains('[') || text.contains('\n') {
        return None;
    }
    let url_start = close + 2;
    let url_end = s[url_start..].find(')')? + url_start;
    let url = &s[url_start..url_end];
    if url.contains(' ') {
        return None;
    }
    Some((text, url, url_end + 1))
}

/// Both parts of an outgoing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingBodies {
    pub text: String,
    pub html: String,
}

/// Builds the text and HTML parts of `draft`, appending the quoted `source` when the draft
/// asks for it. Replies get an attribution line and a blockquote; forwards get a header block
/// and the original in full.
pub fn outgoing_bodies(draft: &Draft, source: Option<&Message>) -> OutgoingBodies {
    let mut text = markdown_to_plain(&draft.body_text);
    let mut html = render_markdown(&draft.body_text);
    let Some(source) = source.filter(|_| draft.include_quote) else {
        return OutgoingBodies { text, html };
    };
    let source_text = source.body.text.clone().or_else(|| source.body.html.as_deref().map(html_to_text)).unwrap_or_else(|| source.snippet.clone());
    let source_html = match (&source.body.html, &source.body.text) {
        (Some(h), _) => clean_only(h, RemoteContent::Allowed),
        (None, Some(t)) => text_to_html(t),
        (None, None) => text_to_html(&source.snippet),
    };
    match draft.kind {
        DraftKind::Forward => {
            let header = forward_header(source);
            text = format!("{text}\n\n{header}\n\n{source_text}");
            let header_html = header
                .lines()
                .map(|l| match l.split_once(": ") {
                    Some((k, v)) if !l.starts_with('-') => format!("<b>{}:</b> {}", escape_html(k), escape_html(v)),
                    _ => escape_html(l),
                })
                .collect::<Vec<_>>()
                .join("<br>");
            html = format!("{html}<br><div class=\"overlook_forward\"><p>{header_html}</p>{source_html}</div>");
        }
        _ => {
            let header = quote_header(source);
            text = format!("{text}\n\n{header}\n{}", quote_text(&source_text));
            html = format!(
                "{html}<br><div class=\"overlook_quote\"><p>{}</p><blockquote style=\"margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex\">{source_html}</blockquote></div>",
                escape_html(&header)
            );
        }
    }
    OutgoingBodies { text, html }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::*;
    use crate::model::{Address, Body};

    #[test]
    fn renders_mail_markdown() {
        let html = render_markdown("Hi **Maya**,\nsee *this* and ~~that~~ at https://ex.com/a\n\n- one\n- two\n\n> quoted");
        assert!(html.contains("<strong>Maya</strong>"), "{html}");
        assert!(html.contains("Hi <strong>Maya</strong>,<br>"), "{html}");
        assert!(html.contains("<em>this</em>") && html.contains("<del>that</del>"), "{html}");
        assert!(html.contains("<a href=\"https://ex.com/a\">https://ex.com/a</a>"), "{html}");
        assert!(html.contains("<ul>") && html.contains("<li>one</li>"), "{html}");
        assert!(html.contains("<blockquote>"), "{html}");
    }

    #[test]
    fn typed_html_is_escaped_not_rendered() {
        let html = render_markdown("<script>alert(1)</script> & <b>x</b>");
        assert!(!html.contains("<script>") && !html.contains("<b>x</b>"), "{html}");
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"), "{html}");
    }

    #[test]
    fn plain_text_drops_marks_keeps_structure() {
        let plain = markdown_to_plain("# Title\n\nHi **Maya**, see [the brief](https://ex.com/b) and `cargo test`.\n- *one*\n> quoted_word_here");
        assert_eq!(plain, "Title\n\nHi Maya, see the brief (https://ex.com/b) and cargo test.\n- one\n> quoted_word_here");
    }

    fn source() -> Message {
        Message {
            id: MessageId::from("a:1"),
            account_id: AccountId::from("a"),
            conversation_id: ConversationId::from("a:t"),
            provider_id: None,
            from: Address::named("Maya Chen", "maya@northstar.example"),
            to: vec![Address::named("Jason", "jason@example.com")],
            cc: vec![],
            bcc: vec![],
            reply_to: None,
            subject: "Launch".into(),
            snippet: "Can we meet?".into(),
            body: Body { text: Some("Can we meet?\nThursday works.".into()), html: Some("<div>Can we meet?<br>Thursday works.</div>".into()) },
            date: 0,
            is_read: true,
            is_starred: false,
            is_outgoing: false,
            label_ids: Default::default(),
            attachments: vec![],
            rfc_message_id: None,
            in_reply_to: None,
            category: None,
            is_important: false,
            focus_override: None,
            invitation: None,
        }
    }

    #[test]
    fn reply_quotes_below_the_body() {
        let mut draft = Draft::new(AccountId::from("a"), DraftKind::Reply, 0);
        draft.body_text = "Thursday it is.".into();
        draft.include_quote = true;
        let out = outgoing_bodies(&draft, Some(&source()));
        assert!(out.text.starts_with("Thursday it is.\n\nOn "), "{}", out.text);
        assert!(out.text.ends_with("wrote:\n> Can we meet?\n> Thursday works."), "{}", out.text);
        assert!(out.html.starts_with("<p>Thursday it is.</p>"), "{}", out.html);
        assert!(out.html.contains("<blockquote") && out.html.contains("Thursday works."), "{}", out.html);
        draft.include_quote = false;
        assert_eq!(outgoing_bodies(&draft, Some(&source())).text, "Thursday it is.");
    }

    #[test]
    fn forward_carries_a_header_block() {
        let mut draft = Draft::new(AccountId::from("a"), DraftKind::Forward, 0);
        draft.body_text = "FYI".into();
        draft.include_quote = true;
        let out = outgoing_bodies(&draft, Some(&source()));
        assert!(out.text.contains("---------- Forwarded message ----------\nFrom: Maya Chen <maya@northstar.example>"), "{}", out.text);
        assert!(out.html.contains("<b>From:</b> Maya Chen &lt;maya@northstar.example&gt;"), "{}", out.html);
        assert!(out.html.contains("Thursday works."));
    }
}
