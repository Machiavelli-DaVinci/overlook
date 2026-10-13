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
    /// The quoted history of a reply, split off so the reader can collapse it.
    pub quoted: Option<String>,
    /// Number of remote resources removed.
    pub blocked_remote: usize,
    /// Plain-text rendering for previews and search (body and quoted history).
    pub text: String,
}

/// Sanitizes `html` for display. Never fails: malformed input yields best-effort output.
/// The ammonia policy shared by the reader and by outgoing quotes.
fn builder(policy: RemoteContent) -> ammonia::Builder<'static> {
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

    builder
}

/// Cleans HTML without the structural pre-pass: scripts, handlers, and disallowed tags are
/// removed, but the quoted history, hidden blocks, and layout stay as they are. Used when an
/// original message is quoted in a reply or forward, where the recipient should get the whole
/// thing.
pub fn clean_only(html: &str, policy: RemoteContent) -> String {
    builder(policy).clean(html).to_string()
}

pub fn sanitize_html(html: &str, policy: RemoteContent) -> Sanitized {
    let prepared = crate::prepass::prepare(html);
    let blocked_remote = if policy == RemoteContent::Blocked {
        count_remote(&prepared.html) + prepared.quoted.as_deref().map(count_remote).unwrap_or(0)
    } else {
        0
    };
    let html = prepared.html.as_str();

    let builder = builder(policy);
    let mut clean = flatten_tables(&builder.clean(html).to_string());
    let mut quoted = prepared.quoted.as_deref().map(|q| flatten_tables(&builder.clean(q).to_string()));
    let mut text = html_to_text(&clean);
    if let Some(q) = &quoted {
        let quoted_text = html_to_text(q);
        if text.trim().is_empty() {
            // Nothing but history: show it inline rather than behind a toggle.
            clean.push_str(q);
            quoted = None;
            text = quoted_text;
        } else if !quoted_text.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(&quoted_text);
        } else {
            quoted = None;
        }
    }
    Sanitized { html: clean, quoted, blocked_remote, text }
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
        if name == "table" && !closing {
            // A real data table (headers, or a consistent small grid of short cells) is kept
            // for the view's table layout; layout tables fall through and are flattened.
            if let Some(block_len) = table_block_len(tag_rest) {
                let block = &tag_rest[..block_len];
                if is_data_table(block) {
                    out.push_str(block);
                    rest = &tag_rest[block_len..];
                    continue;
                }
            }
        }
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

/// Length of the `<table …>…</table>` element starting at `s`, nesting-aware.
fn table_block_len(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(off) = lower[i..].find("<table").or_else(|| lower[i..].find("</table")).map(|o| o) {
        // Take whichever comes first.
        let open = lower[i..].find("<table").map(|o| i + o);
        let close = lower[i..].find("</table").map(|o| i + o);
        let _ = off;
        let (pos, is_open) = match (open, close) {
            (Some(o), Some(c)) if o < c => (o, true),
            (_, Some(c)) => (c, false),
            (Some(o), None) => (o, true),
            (None, None) => return None,
        };
        let end = lower[pos..].find('>').map(|e| pos + e + 1)?;
        if is_open {
            depth += 1;
        } else {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(end);
            }
        }
        i = end;
    }
    None
}

/// Whether a table carries data rather than layout: it has header cells, or it is a small
/// consistent grid of short texts with no nested tables or images.
fn is_data_table(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    if lower.matches("<table").count() > 1 || lower.contains("<img") {
        return false;
    }
    let rows: Vec<&str> = lower.split("<tr").skip(1).collect();
    if rows.len() < 2 {
        return false;
    }
    let mut cell_counts = Vec::new();
    let mut longest = 0usize;
    let mut total = 0usize;
    let mut cells = 0usize;
    for row in &rows {
        // Spanning cells count for the columns they cover.
        let mut n = 0usize;
        for cell in row.split("<td").chain(row.split("<th")).skip(1) {
            let tag = cell.split('>').next().unwrap_or("");
            let span = tag.split("colspan=").nth(1).map(|v| v.trim_start_matches(['"', '\'']).chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).and_then(|d| d.parse::<usize>().ok()).unwrap_or(1);
            n += span.max(1);
        }
        cell_counts.push(n);
        for cell in row.split("<td").chain(row.split("<th")).skip(1) {
            let text = html_to_text(cell.split_once('>').map(|(_, t)| t).unwrap_or(""));
            let len = text.trim().chars().count();
            longest = longest.max(len);
            total += len;
            cells += 1;
        }
    }
    let has_header = lower.contains("<th");
    let first = cell_counts[0];
    let consistent = first >= 2 && first <= 8 && cell_counts.iter().all(|&n| n == first || n == 0);
    let short = cells > 0 && longest <= 160 && total / cells <= 60;
    (has_header && consistent) || (consistent && short)
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

/// Converts plain text to safe HTML for display in the HTML view.
///
/// Blank lines separate paragraphs; single newlines become `<br>`. Lines quoted with `>`
/// are grouped into (nested) blockquotes with the markers removed, the way a mail client
/// shows a plain-text reply. Bare URLs become links.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut open_paragraph = false;
    let mut first_in_block = true;
    let mut pending_blank = false;

    for raw in text.lines() {
        let (line_depth, body) = strip_quote_markers(raw);
        if body.trim().is_empty() {
            pending_blank = true;
            continue;
        }
        if line_depth != depth {
            if open_paragraph {
                out.push_str("</p>");
                open_paragraph = false;
            }
            while depth > line_depth {
                out.push_str("</blockquote>");
                depth -= 1;
            }
            while depth < line_depth {
                out.push_str("<blockquote>");
                depth += 1;
            }
            first_in_block = true;
            pending_blank = false;
        }
        if depth == 0 {
            if pending_blank && open_paragraph {
                out.push_str("</p>");
                open_paragraph = false;
            }
            if !open_paragraph {
                out.push_str("<p>");
                open_paragraph = true;
                first_in_block = true;
            }
            if !first_in_block {
                out.push_str("<br>");
            }
        } else if !first_in_block {
            out.push_str(if pending_blank { "<br><br>" } else { "<br>" });
        }
        out.push_str(&linkify_escaped(body));
        first_in_block = false;
        pending_blank = false;
    }
    if open_paragraph {
        out.push_str("</p>");
    }
    while depth > 0 {
        out.push_str("</blockquote>");
        depth -= 1;
    }
    out
}

/// Counts the leading `>` markers of a quoted line and returns the text after them.
fn strip_quote_markers(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line.trim_start();
    while let Some(after) = rest.strip_prefix('>') {
        depth += 1;
        rest = after.strip_prefix(' ').unwrap_or(after);
    }
    (depth, rest)
}

/// Escapes text for HTML, turning bare URLs into links.
fn linkify_escaped(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((start, len)) = crate::prepass::find_url(rest) {
        out.push_str(&escape_html(&rest[..start]));
        let url = &rest[start..start + len];
        let href = if url.starts_with("www.") { format!("https://{url}") } else { url.to_owned() };
        out.push_str(&format!("<a href=\"{}\">{}</a>", escape_html(&href), escape_html(url)));
        rest = &rest[start + len..];
    }
    out.push_str(&escape_html(rest));
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
        // A header row makes it a data table, kept for the view's grid layout.
        let s = sanitize_html(
            "<table><tr><th>Area</th><th>Done</th></tr><tr><td>Design</td><td>5</td></tr></table>",
            RemoteContent::Blocked,
        );
        assert!(s.html.starts_with("<table>") && s.html.contains("<th>Area</th>"), "{}", s.html);
        // Layout tables (nested, long prose) become block flow with gaps between cells.
        let nested = sanitize_html("<table><tr><td><p>Hello there, this is a long paragraph of newsletter copy that runs on well past the length of any data cell and keeps going.</p><table><tr><td><p>Another long paragraph of copy inside the nested layout table that is also far too long for a data cell.</p></td><td>b</td></tr></table></td></tr></table>", RemoteContent::Blocked);
        assert!(!nested.html.contains("<table"), "{}", nested.html);
        assert!(nested.html.contains("<p>Hello there") && nested.html.contains("\u{a0}\u{a0}\u{a0}\u{a0}b"), "{}", nested.html);
    }

    #[test]
    fn data_tables_are_kept_and_layout_tables_flattened() {
        let invoice = r#"<table border="1"><thead><tr><th>Item</th><th>Qty</th></tr></thead><tbody><tr><td>Lamp</td><td>2</td></tr><tr><td>Cable</td><td>1</td></tr></tbody></table>"#;
        let s = sanitize_html(invoice, RemoteContent::Blocked);
        assert!(s.html.contains("<table") && s.html.contains("<th>Item</th>"), "{}", s.html);
        let layout = r#"<table width="600"><tr><td><p>A long paragraph of newsletter copy that goes on and on and on and on and on and on, well past the length any data cell would ever have in a real table of figures.</p><table><tr><td>inner</td></tr></table></td></tr></table>"#;
        let s = sanitize_html(layout, RemoteContent::Blocked);
        assert!(!s.html.contains("<table"), "{}", s.html);
        assert!(s.html.contains("inner"));
    }

    #[test]
    fn breaks_are_kept_for_the_view() {
        // The HTML view (vendored gpui-base) renders <br> as a hard line break, so the
        // sanitizer leaves it alone; the text extraction keeps it as a newline.
        let s = sanitize_html("<p>Thanks,<br>Maya<br><small>Design Director</small></p><div>a<br>b</div>", RemoteContent::Blocked);
        assert_eq!(s.html, "<p>Thanks,<br>Maya<br><small>Design Director</small></p><div>a<br>b</div>");
        assert_eq!(s.text, "Thanks,\nMaya\nDesign Director\n\na\nb");
    }

    #[test]
    fn gmail_div_bodies_keep_their_lines() {
        // Gmail composes mail as nested divs with <br> between lines; every break survives.
        let s = sanitize_html("<div dir=\"ltr\"><div dir=\"auto\">Hey,<br><br>Step 1<br>- one<br>- two<br><br>Bye</div></div>", RemoteContent::Blocked);
        assert_eq!(s.html, "<div dir=\"ltr\"><div dir=\"auto\">Hey,<br><br>Step 1<br>- one<br>- two<br><br>Bye</div></div>");
    }

    #[test]
    fn text_round_trips_to_html() {
        let html = text_to_html("Hello <you>\nsecond line\n\n> quoted one\n> quoted two\n>\n> > nested\n\nAfter https://ex.com/a.");
        assert_eq!(
            html,
            "<p>Hello &lt;you&gt;<br>second line</p><blockquote>quoted one<br>quoted two<blockquote>nested</blockquote></blockquote><p>After <a href=\"https://ex.com/a\">https://ex.com/a</a>.</p>"
        );
    }
}
