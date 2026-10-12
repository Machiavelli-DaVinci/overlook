//! Sanitization of received HTML.
//!
//! Received mail is untrusted. The sanitizer removes scripts, styles, forms, frames, event
//! handlers, and (by default) every remote resource. The output is safe to hand to the
//! native HTML text view, which executes nothing, but the policy is enforced here so that
//! no renderer ever sees the raw markup.

use std::collections::HashSet;

/// Remote-content policy for a particular rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RemoteContent {
    /// Block remote images and other resources (default).
    #[default]
    Blocked,
    /// The user chose to load remote content for this message.
    Allowed,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Sanitized {
    pub html: String,
    /// Number of remote resources removed.
    pub blocked_remote: usize,
    /// Plain-text rendering for previews and search.
    pub text: String,
}

/// Sanitizes `html` for display. Never fails: malformed input yields best-effort output.
pub fn sanitize_html(html: &str, policy: RemoteContent) -> Sanitized {
    let blocked_remote = if policy == RemoteContent::Blocked { count_remote(html) } else { 0 };

    let mut builder = ammonia::Builder::default();
    let tags: HashSet<&str> = [
        "a", "abbr", "b", "blockquote", "br", "caption", "center", "code", "dd", "del", "div",
        "dl", "dt", "em", "font", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "ins",
        "kbd", "li", "ol", "p", "pre", "q", "s", "small", "span", "strike", "strong", "sub",
        "sup", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "u", "ul",
    ]
    .into_iter()
    .collect();
    builder.tags(tags);
    builder.rm_tags(["script", "style", "iframe", "object", "embed", "form", "input", "button", "link", "meta", "svg", "math", "video", "audio", "base"]);
    builder.clean_content_tags(["script", "style", "iframe", "object", "embed", "svg", "math", "video", "audio", "noscript", "template"].into_iter().collect());
    builder.generic_attributes(["title", "dir", "lang"].into_iter().collect());
    builder.add_tag_attributes("a", ["href"]);
    builder.add_tag_attributes("img", ["src", "alt", "width", "height"]);
    builder.add_tag_attributes("td", ["colspan", "rowspan"]);
    builder.add_tag_attributes("th", ["colspan", "rowspan"]);
    builder.add_tag_attributes("ol", ["start"]);
    builder.link_rel(Some("noopener noreferrer"));
    builder.url_schemes(["http", "https", "mailto"].into_iter().collect());
    builder.strip_comments(true);
    match policy {
        RemoteContent::Blocked => {
            // Only inline `cid:` references survive; everything remote is removed.
            builder.url_schemes(["http", "https", "mailto", "cid"].into_iter().collect());
            builder.attribute_filter(|element, attribute, value| {
                if element == "img" && attribute == "src" && !value.starts_with("cid:") {
                    None
                } else {
                    Some(value.into())
                }
            });
        }
        RemoteContent::Allowed => {
            builder.url_schemes(["http", "https", "mailto", "cid"].into_iter().collect());
        }
    }

    let clean = normalize_breaks(&flatten_tables(&builder.clean(html).to_string()));
    let text = html_to_text(&clean);
    Sanitized { html: clean, blocked_remote, text }
}

/// Rewrites tables into block flow: each row becomes a block, cells are separated by a
/// wide gap. Email relies on nested layout tables that the native HTML view cannot lay
/// out yet; without this step adjacent cells run together. Column alignment of real
/// data tables is lost, which is a known foundation gap.
pub fn flatten_tables(html: &str) -> String {
    const CELL_GAP: &str = "\u{a0}\u{a0}\u{a0}\u{a0}";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut cell_open = false;
    let mut first_cell_in_row = true;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tag_rest = &rest[start..];
        let Some(end) = tag_rest.find('>') else {
            out.push_str(tag_rest);
            return out;
        };
        let tag = &tag_rest[..=end];
        let lower = tag.to_ascii_lowercase();
        let name: String = lower
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = lower.starts_with("</");
        match name.as_str() {
            "table" | "tbody" | "thead" | "tfoot" | "caption" => {
                if name == "caption" {
                    out.push_str(if closing { "</p>" } else { "<p>" });
                }
            }
            "tr" => {
                if closing {
                    if cell_open {
                        cell_open = false;
                    }
                    out.push_str("</div>");
                } else {
                    out.push_str("<div>");
                    first_cell_in_row = true;
                }
            }
            "td" | "th" => {
                if closing {
                    cell_open = false;
                    if name == "th" {
                        out.push_str("</strong>");
                    }
                } else {
                    if !first_cell_in_row {
                        out.push_str(CELL_GAP);
                    }
                    first_cell_in_row = false;
                    cell_open = true;
                    if name == "th" {
                        out.push_str("<strong>");
                    }
                }
            }
            _ => out.push_str(tag),
        }
        rest = &tag_rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Turns `<br>` inside a paragraph into a paragraph boundary.
///
/// The native HTML view treats `<br>` as a block-level break and ignores it inside a
/// paragraph, which swallows the line breaks in signatures and addresses. Splitting the
/// paragraph keeps every line visible. Breaks outside paragraphs are left alone.
pub fn normalize_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 32);
    let mut rest = html;
    let mut in_paragraph = false;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tag_rest = &rest[start..];
        let Some(end) = tag_rest.find('>') else {
            out.push_str(tag_rest);
            return out;
        };
        let tag = &tag_rest[..=end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<p>") || lower.starts_with("<p ") {
            in_paragraph = true;
            out.push_str(tag);
        } else if lower.starts_with("</p>") {
            in_paragraph = false;
            out.push_str(tag);
        } else if in_paragraph && (lower.starts_with("<br>") || lower.starts_with("<br/>") || lower.starts_with("<br ")) {
            out.push_str("</p><p>");
        } else {
            out.push_str(tag);
        }
        rest = &tag_rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn count_remote(html: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let mut count = 0;
    for needle in ["src=\"http", "src='http", "src=http"] {
        count += lower.matches(needle).count();
    }
    count
}

/// Very small HTML-to-text conversion for previews, quoting, and the search index. Not a
/// layout engine: it preserves paragraph breaks and list markers and drops tags.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                let closing = tag.starts_with('/');
                match name.as_str() {
                    "br" => out.push('\n'),
                    "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote"
                    | "pre" | "table" | "ul" | "ol" => {
                        if !out.ends_with("\n\n") && !out.is_empty() {
                            if out.ends_with('\n') {
                                out.push('\n');
                            } else {
                                out.push_str("\n\n");
                            }
                        }
                    }
                    "li" if !closing => {
                        if !out.ends_with('\n') && !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str("• ");
                    }
                    "td" | "th" if closing => out.push('\t'),
                    _ => {}
                }
                tag.clear();
            } else {
                tag.push(c);
            }
            continue;
        }
        match c {
            '<' => in_tag = true,
            '&' => {
                let mut entity = String::new();
                while let Some(&n) = chars.peek() {
                    if n == ';' || entity.len() > 8 {
                        break;
                    }
                    entity.push(n);
                    chars.next();
                }
                if chars.peek() == Some(&';') {
                    chars.next();
                }
                out.push_str(&decode_entity(&entity));
            }
            _ => out.push(c),
        }
    }
    collapse_whitespace(&out)
}

fn decode_entity(entity: &str) -> String {
    match entity {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" | "#39" => "'".into(),
        "nbsp" => " ".into(),
        "mdash" => "—".into(),
        "ndash" => "–".into(),
        "hellip" => "…".into(),
        "rsquo" => "’".into(),
        "lsquo" => "‘".into(),
        "ldquo" => "“".into(),
        "rdquo" => "”".into(),
        "copy" => "©".into(),
        other => {
            if let Some(num) = other.strip_prefix('#') {
                let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num.parse().ok()
                };
                if let Some(ch) = code.and_then(char::from_u32) {
                    return ch.to_string();
                }
            }
            format!("&{other};")
        }
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0;
    let mut space = false;
    for c in s.chars() {
        match c {
            '\n' => {
                newlines += 1;
                space = false;
                if newlines <= 2 {
                    out.push('\n');
                }
            }
            c if c.is_whitespace() => {
                if !space && newlines == 0 {
                    out.push(' ');
                }
                space = true;
            }
            c => {
                newlines = 0;
                space = false;
                out.push(c);
            }
        }
    }
    out.trim().to_owned()
}

/// Converts plain text to safe HTML paragraphs for display in the HTML view.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::new();
    for para in text.split("\n\n") {
        let escaped = escape_html(para).replace('\n', "<br>");
        if para.trim_start().starts_with('>') {
            out.push_str("<blockquote>");
            out.push_str(&escaped);
            out.push_str("</blockquote>");
        } else {
            out.push_str("<p>");
            out.push_str(&escaped);
            out.push_str("</p>");
        }
    }
    out
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_handlers() {
        let s = sanitize_html(
            r#"<p onclick="x()">Hi<script>alert(1)</script></p><iframe src="https://evil"></iframe><a href="javascript:alert(1)">x</a>"#,
            RemoteContent::Blocked,
        );
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("iframe"));
        assert!(!s.html.contains("javascript:"));
        assert!(s.html.contains("<p>Hi</p>"));
    }

    #[test]
    fn blocks_remote_images_by_default() {
        let html = r#"<img src="https://tracker.example/pixel.gif"><img src="cid:logo">"#;
        let blocked = sanitize_html(html, RemoteContent::Blocked);
        assert_eq!(blocked.blocked_remote, 1);
        assert!(!blocked.html.contains("tracker.example"));
        assert!(blocked.html.contains("cid:logo"));
        let allowed = sanitize_html(html, RemoteContent::Allowed);
        assert!(allowed.html.contains("tracker.example"));
    }

    #[test]
    fn text_extraction_keeps_structure() {
        let text = html_to_text("<p>Hi&nbsp;Jason,</p><ul><li>One</li><li>Two &amp; three</li></ul><p>Bye</p>");
        assert_eq!(text, "Hi Jason,\n\n• One\n• Two & three\n\nBye");
    }

    #[test]
    fn tables_become_rows_with_gaps() {
        let s = sanitize_html(
            "<table><tr><th>Area</th><th>Done</th></tr><tr><td>Design</td><td>5</td></tr></table>",
            RemoteContent::Blocked,
        );
        assert_eq!(
            s.html,
            "<div><strong>Area</strong>\u{a0}\u{a0}\u{a0}\u{a0}<strong>Done</strong></div><div>Design\u{a0}\u{a0}\u{a0}\u{a0}5</div>"
        );
        let nested = sanitize_html("<table><tr><td><p>Hello</p><table><tr><td>a</td><td>b</td></tr></table></td></tr></table>", RemoteContent::Blocked);
        assert_eq!(nested.html, "<div><p>Hello</p><div>a\u{a0}\u{a0}\u{a0}\u{a0}b</div></div>");
    }

    #[test]
    fn breaks_inside_paragraphs_become_paragraphs() {
        let s = sanitize_html("<p>Thanks,<br>Maya<br><small>Design Director</small></p><div>a<br>b</div>", RemoteContent::Blocked);
        assert_eq!(s.html, "<p>Thanks,</p><p>Maya</p><p><small>Design Director</small></p><div>a<br>b</div>");
        assert_eq!(s.text, "Thanks,\n\nMaya\n\nDesign Director\n\na\nb");
    }

    #[test]
    fn text_round_trips_to_html() {
        let html = text_to_html("Hello <you>\n\n> quoted");
        assert_eq!(html, "<p>Hello &lt;you&gt;</p><blockquote>&gt; quoted</blockquote>");
    }
}
