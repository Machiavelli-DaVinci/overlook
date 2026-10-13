//! Live styling for the composer: the body is Markdown-flavoured text, and this highlighter
//! shows it formatted while the marks stay visible and editable. Bold reads bold, links are
//! underlined in the accent colour, list markers and quotes are tinted, and the marks
//! themselves fade back so the text is what the eye lands on.

use gpui_kit::component::input::{EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{px, Context, FontStyle, FontWeight, HighlightStyle, Hsla, SharedString, StrikethroughStyle, UnderlineStyle, Window};
use std::ops::Range;

pub const LANGUAGE: &str = "mail";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A formatting mark such as `**` or `[`.
    Marker,
    Bold,
    Italic,
    Strike,
    Code,
    /// The visible text of a `[text](url)` link.
    LinkText,
    /// A URL, bare or inside a link.
    Url,
    Heading,
    Quote,
    ListMarker,
}

pub struct MailHighlighter {
    spans: Vec<(Range<usize>, Kind)>,
    accent: Hsla,
    muted: Hsla,
    code_bg: Hsla,
}

impl MailHighlighter {
    pub fn new(accent: Hsla, muted: Hsla, code_bg: Hsla) -> Self {
        Self { spans: Vec::new(), accent, muted, code_bg }
    }

    fn style_for(&self, kind: Kind) -> HighlightStyle {
        let mut s = HighlightStyle::default();
        match kind {
            Kind::Marker => s.fade_out = Some(0.6),
            Kind::Bold | Kind::Heading => s.font_weight = Some(FontWeight::BOLD),
            Kind::Italic => s.font_style = Some(FontStyle::Italic),
            Kind::Strike => s.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: None }),
            Kind::Code => s.background_color = Some(self.code_bg),
            Kind::LinkText | Kind::Url => {
                s.color = Some(self.accent);
                s.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(self.accent), wavy: false });
            }
            Kind::Quote => s.color = Some(self.muted),
            Kind::ListMarker => {
                s.color = Some(self.accent);
                s.font_weight = Some(FontWeight::BOLD);
            }
        }
        s
    }
}

impl InputHighlighter for MailHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(&mut self, _edit: Option<InputEdit>, text: &Rope, _folding: bool, _window: &mut Window, _cx: &mut Context<EditorState>) {
        // Mail bodies are small; a full rescan per edit is cheaper than being clever.
        self.spans = scan(&text.to_string());
    }

    fn styles(&self, range: &Range<usize>, _resolver: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut out = Vec::new();
        let mut at = range.start;
        for (span, kind) in &self.spans {
            if span.end <= range.start {
                continue;
            }
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start > at {
                out.push((at..start, HighlightStyle::default()));
            }
            out.push((start..end, self.style_for(*kind)));
            at = end;
        }
        if at < range.end {
            out.push((at..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// Byte-offset spans for the whole text, non-overlapping and in order.
pub fn scan(text: &str) -> Vec<(Range<usize>, Kind)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        scan_line(line.trim_end_matches('\n'), offset, &mut spans);
        offset += line.len();
    }
    spans
}

/// The length of a list marker (`- `, `* `, `+ `, `1. `, `12) `) at the start of `s`, if any.
pub fn list_marker_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return Some(2);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 4 && b.len() > digits + 1 && matches!(b[digits], b'.' | b')') && b[digits + 1] == b' ' {
        return Some(digits + 2);
    }
    None
}

fn scan_line(line: &str, base: usize, spans: &mut Vec<(Range<usize>, Kind)>) {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let mut pos = indent;
    // Headings and quotes style the whole line and skip inline marks.
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        spans.push((base + pos..base + pos + hashes + 1, Kind::Marker));
        spans.push((base + pos + hashes + 1..base + line.len(), Kind::Heading));
        return;
    }
    if rest.starts_with('>') {
        spans.push((base + pos..base + line.len(), Kind::Quote));
        return;
    }
    if let Some(n) = list_marker_len(rest) {
        spans.push((base + pos..base + pos + n, Kind::ListMarker));
        pos += n;
    }
    scan_inline(line, pos, base, spans);
}

fn scan_inline(line: &str, mut i: usize, base: usize, spans: &mut Vec<(Range<usize>, Kind)>) {
    let bytes = line.as_bytes();
    let mut push = |r: Range<usize>, k: Kind| spans.push((base + r.start..base + r.end, k));
    while i < line.len() {
        let rest = &line[i..];
        // Inline code: no marks are read inside it.
        if rest.starts_with('`') {
            if let Some(close) = rest[1..].find('`') {
                push(i..i + 1, Kind::Marker);
                push(i + 1..i + 1 + close, Kind::Code);
                push(i + 1 + close..i + 2 + close, Kind::Marker);
                i += close + 2;
                continue;
            }
        }
        for (mark, kind) in [("**", Kind::Bold), ("__", Kind::Bold), ("~~", Kind::Strike)] {
            if rest.starts_with(mark) && !rest[2..].starts_with(' ') {
                if let Some(close) = rest[2..].find(mark).filter(|c| *c > 0) {
                    push(i..i + 2, Kind::Marker);
                    push(i + 2..i + 2 + close, kind);
                    push(i + 2 + close..i + 4 + close, Kind::Marker);
                    i += close + 4;
                    continue;
                }
            }
        }
        if i < line.len() && &line[i..] != rest {
            continue; // advanced by the loop above
        }
        if rest.starts_with('[') {
            if let Some((text, url, len)) = overlook_core::compose::parse_link(rest) {
                push(i..i + 1, Kind::Marker);
                push(i + 1..i + 1 + text.len(), Kind::LinkText);
                push(i + 1 + text.len()..i + 3 + text.len(), Kind::Marker);
                push(i + 3 + text.len()..i + 3 + text.len() + url.len(), Kind::Url);
                push(i + len - 1..i + len, Kind::Marker);
                i += len;
                continue;
            }
        }
        if (rest.starts_with('*') || rest.starts_with('_')) && !rest[1..].starts_with(' ') && !rest[1..].starts_with(&rest[..1]) {
            let marker = bytes[i];
            let boundary_before = marker == b'*' || i == 0 || !line[..i].ends_with(|c: char| c.is_alphanumeric());
            if boundary_before {
                if let Some(close) = rest[1..].find(marker as char).filter(|c| *c > 0) {
                    let after = &rest[close + 2..];
                    if marker == b'*' || !after.starts_with(|c: char| c.is_alphanumeric()) {
                        push(i..i + 1, Kind::Marker);
                        push(i + 1..i + 1 + close, Kind::Italic);
                        push(i + 1 + close..i + 2 + close, Kind::Marker);
                        i += close + 2;
                        continue;
                    }
                }
            }
        }
        if rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.") {
            if let Some((0, len)) = overlook_core::prepass::find_url(rest) {
                push(i..i + len, Kind::Url);
                i += len;
                continue;
            }
        }
        i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, Kind)> {
        scan(text).into_iter().map(|(r, k)| (text[r].to_string(), k)).collect()
    }

    #[test]
    fn marks_bold_italic_links_and_lists() {
        let k = kinds("- **Bold** and *it* at https://a.b/c\n> quoted\n# Head\nsnake_case_word [t](u)");
        assert!(k.contains(&("- ".into(), Kind::ListMarker)));
        assert!(k.contains(&("Bold".into(), Kind::Bold)) && k.contains(&("**".into(), Kind::Marker)));
        assert!(k.contains(&("it".into(), Kind::Italic)));
        assert!(k.contains(&("https://a.b/c".into(), Kind::Url)));
        assert!(k.contains(&("> quoted".into(), Kind::Quote)));
        assert!(k.contains(&("Head".into(), Kind::Heading)));
        assert!(!k.iter().any(|(s, k)| *k == Kind::Italic && s.contains("case")));
        assert!(k.contains(&("t".into(), Kind::LinkText)) && k.contains(&("u".into(), Kind::Url)));
    }

    #[test]
    fn styles_cover_the_requested_range() {
        let mut h = MailHighlighter::new(Hsla::default(), Hsla::default(), Hsla::default());
        h.spans = scan("ab **cd** ef");
        let runs = h.styles(&(0..12), &NoStyles);
        assert_eq!(runs.first().unwrap().0.start, 0);
        assert_eq!(runs.last().unwrap().0.end, 12);
        let total: usize = runs.iter().map(|(r, _)| r.len()).sum();
        assert_eq!(total, 12);
    }

    struct NoStyles;
    impl HighlightStyleResolver for NoStyles {
        fn style(&self, _: &str) -> Option<HighlightStyle> {
            None
        }
    }
}
