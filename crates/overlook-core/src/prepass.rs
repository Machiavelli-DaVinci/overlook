//! Structural clean-up of received HTML, run before sanitizing.
//!
//! Mail HTML carries things a browser hides or a client is expected to tidy: preheader text
//! hidden with `display:none`, tracking pixels, `<title>` text, spacer blocks such as
//! Gmail's `<div><br></div>` and Outlook's `<p>&nbsp;</p>`, `<center>` wrappers, and the
//! quoted history of a reply. The native HTML view has no CSS, so these are resolved on
//! the DOM here. The result is still untrusted HTML; `sanitize_html` cleans it afterwards.

use html5ever::serialize::{serialize, SerializeOpts, TraversalScope};
use html5ever::tendril::TendrilSink;
use html5ever::{namespace_url, ns, parse_document, Attribute, LocalName, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use std::cell::RefCell;
use std::rc::Rc;

/// The message split into what the reader shows and the quoted history, if any.
pub struct Prepared {
    pub html: String,
    pub quoted: Option<String>,
}

/// Elements removed with their content.
const REMOVE: &[&str] = &["head", "title", "style", "script", "meta", "link", "noscript", "template", "iframe", "object", "embed", "svg", "math", "base"];
/// Inline wrappers that carry no content of their own.
const INLINE_WRAPPERS: &[&str] = &["span", "font", "b", "i", "u", "strong", "em", "small", "o:p", "a", "sup", "sub"];
/// Elements that start on their own line, so a `<br>` next to them adds nothing.
const BLOCKS: &[&str] = &["div", "p", "table", "ul", "ol", "li", "blockquote", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "pre", "dl", "center"];

pub fn prepare(html: &str) -> Prepared {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let doc = dom.document.clone();
    let cleaned = clean_children(&doc, &Flags::default());
    set_children(&doc, cleaned);
    let quoted = split_quote(&doc).map(|nodes| {
        let holder = element("div", vec![]);
        set_children(&holder, nodes);
        serialize_children(&holder)
    });
    Prepared { html: serialize_children(&doc), quoted }
}

#[derive(Clone, Copy, Default)]
struct Flags {
    in_pre: bool,
    no_links: bool,
}

fn clean_children(node: &Handle, flags: &Flags) -> Vec<Handle> {
    let mut out = Vec::new();
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if flags.in_pre {
                    out.extend(preformatted_nodes(&text));
                } else if flags.no_links {
                    out.push(child.clone());
                } else {
                    out.extend(linkified_nodes(&text, child));
                }
            }
            NodeData::Element { name, attrs, .. } => {
                let local = name.local.to_string();
                let attrs_vec = attrs.borrow().clone();
                if REMOVE.contains(&local.as_str()) || is_hidden(&local, &attrs_vec) {
                    continue;
                }
                let mut inner = *flags;
                inner.in_pre |= local == "pre";
                inner.no_links |= matches!(local.as_str(), "a" | "pre" | "code");
                let children = clean_children(child, &inner);
                let (new_name, bold) = match local.as_str() {
                    "center" => ("div", false),
                    "dt" => ("div", true),
                    "dd" => ("div", false),
                    _ => (local.as_str(), false),
                };
                if matches!(new_name, "p" | "div") && is_blank(&children) {
                    continue;
                }
                if new_name != local || bold {
                    // rcdom's Drop empties the children of every node beneath a dropped
                    // node, including ones that now live under `fresh`; detach them first.
                    child.children.borrow_mut().clear();
                    let fresh = element(new_name, attrs_vec);
                    let children = if bold {
                        let b = element("b", vec![]);
                        set_children(&b, children);
                        vec![b]
                    } else {
                        children
                    };
                    set_children(&fresh, children);
                    out.push(fresh);
                } else {
                    set_children(child, children);
                    out.push(child.clone());
                }
            }
            NodeData::Document => {}
            // Comments (including Outlook's conditional comments), doctypes, PIs.
            _ => {}
        }
    }
    trim_breaks(out)
}

fn is_named(node: &Handle, names: &[&str]) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if names.contains(&&*name.local))
}

/// Drops `<br>`s that only add blank lines: at the start of a block, at its end, and
/// immediately before a block-level sibling, which starts on a new line anyway.
fn trim_breaks(children: Vec<Handle>) -> Vec<Handle> {
    let is_content = |n: &Handle| match &n.data {
        NodeData::Text { contents } => !is_blank_text(&contents.borrow()),
        NodeData::Element { .. } => !is_named(n, &["br"]),
        _ => false,
    };
    let mut out: Vec<Handle> = Vec::with_capacity(children.len());
    for (i, child) in children.iter().enumerate() {
        if is_named(child, &["br"]) {
            let before = out.iter().any(is_content);
            let next = children[i + 1..].iter().find(|n| match &n.data {
                NodeData::Text { contents } => !is_blank_text(&contents.borrow()),
                NodeData::Element { .. } => !is_named(n, &["br"]),
                _ => false,
            });
            let followed_by_block = next.is_none_or(|n| is_named(n, BLOCKS));
            if !before || followed_by_block {
                continue;
            }
        }
        out.push(child.clone());
    }
    out
}

fn element(name: &str, attrs: Vec<Attribute>) -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from(name)),
        attrs: RefCell::new(attrs),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

fn text_node(text: &str) -> Handle {
    Node::new(NodeData::Text { contents: RefCell::new(text.into()) })
}

fn set_children(parent: &Handle, children: Vec<Handle>) {
    for child in &children {
        child.parent.set(Some(Rc::downgrade(parent)));
    }
    *parent.children.borrow_mut() = children;
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    attrs.iter().find(|a| &*a.name.local == name).map(|a| &*a.value)
}

/// Whether an element would be invisible in a browser, or is a tracking pixel.
fn is_hidden(local: &str, attrs: &[Attribute]) -> bool {
    if attr(attrs, "hidden").is_some() {
        return true;
    }
    if let Some(class) = attr(attrs, "class") {
        let class = class.to_ascii_lowercase();
        if class.contains("preheader") || class.contains("preview-text") {
            return true;
        }
    }
    if local == "img" {
        let dim = |n: &str| attr(attrs, n).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        if let (Some(w), Some(h)) = (dim("width"), dim("height")) {
            if w <= 1.0 && h <= 1.0 {
                return true;
            }
        }
    }
    let Some(style) = attr(attrs, "style") else { return false };
    for decl in style.split(';') {
        let Some((key, value)) = decl.split_once(':') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_ascii_lowercase();
        let value = value.trim_end_matches("!important").trim();
        let zero = |v: &str| matches!(v, "0" | "0px" | "0pt" | "0em" | "0%" | "0.0");
        let hidden = match key.as_str() {
            "display" => value == "none",
            "visibility" => value == "hidden",
            "opacity" => zero(value),
            "mso-hide" => value == "all",
            "max-height" => zero(value),
            "font-size" => zero(value) || matches!(value, "1px" | "1pt"),
            _ => false,
        };
        if hidden {
            return true;
        }
    }
    false
}

/// Whitespace, non-breaking spaces, and the invisible characters preheaders pad with.
fn is_blank_text(s: &str) -> bool {
    s.chars().all(|c| c.is_whitespace() || matches!(c, '\u{a0}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{034f}' | '\u{feff}' | '\u{2007}' | '\u{202f}'))
}

/// A block that only spaces things out: text-free apart from breaks and empty inline wrappers.
fn is_blank(children: &[Handle]) -> bool {
    children.iter().all(|c| match &c.data {
        NodeData::Text { contents } => is_blank_text(&contents.borrow()),
        NodeData::Element { name, .. } => {
            let local = name.local.to_string();
            local == "br" || (INLINE_WRAPPERS.contains(&local.as_str()) && is_blank(&c.children.borrow()))
        }
        _ => true,
    })
}

/// Preformatted text as nodes that survive HTML whitespace collapsing: `<br>` for newlines
/// and non-breaking spaces for indentation. The HTML view turns them back into a code block.
fn preformatted_nodes(text: &str) -> Vec<Handle> {
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(element("br", vec![]));
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let mut s = String::with_capacity(line.len());
        for _ in 0..indent {
            s.push('\u{a0}');
        }
        s.push_str(&line[indent..]);
        if !s.is_empty() {
            out.push(text_node(&s));
        }
    }
    out
}

/// Splits a text node around bare URLs, turning each into a link.
fn linkified_nodes(text: &str, original: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut any = false;
    while let Some((start, len)) = find_url(rest) {
        any = true;
        if start > 0 {
            out.push(text_node(&rest[..start]));
        }
        let url = &rest[start..start + len];
        let href = if url.starts_with("www.") { format!("https://{url}") } else { url.to_owned() };
        let a = element("a", vec![Attribute { name: QualName::new(None, ns!(), LocalName::from("href")), value: href.into() }]);
        set_children(&a, vec![text_node(url)]);
        out.push(a);
        rest = &rest[start + len..];
    }
    if !any {
        return vec![original.clone()];
    }
    if !rest.is_empty() {
        out.push(text_node(rest));
    }
    out
}

/// Position and length of the first bare URL in `s`.
pub fn find_url(s: &str) -> Option<(usize, usize)> {
    let mut best: Option<usize> = None;
    for needle in ["https://", "http://", "www."] {
        if let Some(i) = s.find(needle) {
            // A word boundary before it; "www." must not be part of a longer token.
            let boundary = i == 0 || !s[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '/' || c == '.' || c == '@');
            if boundary && best.is_none_or(|b| i < b) {
                best = Some(i);
            }
        }
    }
    let start = best?;
    let tail = &s[start..];
    let mut end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '\u{a0}')).unwrap_or(tail.len());
    // Trailing punctuation belongs to the sentence, not the link.
    while end > 0 {
        let c = tail[..end].chars().next_back().unwrap();
        let unbalanced_paren = c == ')' && tail[..end].matches('(').count() < tail[..end].matches(')').count();
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ']') || unbalanced_paren {
            end -= c.len_utf8();
        } else {
            break;
        }
    }
    let url = &tail[..end];
    let min = if url.starts_with("www.") { 5 } else { 9 };
    (end >= min && url.contains('.')).then_some((start, end))
}

// ------------------------------------------------------------------ quoted history

/// Detaches the quoted history (and whatever follows it) from the message body.
fn split_quote(node: &Handle) -> Option<Vec<Handle>> {
    let children: Vec<Handle> = node.children.borrow().clone();
    for (i, child) in children.iter().enumerate() {
        if is_quote_start(child) {
            let (keep, quoted) = children.split_at(i);
            let quoted = quoted.to_vec();
            set_children(node, keep.to_vec());
            return Some(quoted);
        }
        if matches!(child.data, NodeData::Element { .. }) {
            if let Some(found) = split_quote(child) {
                return Some(found);
            }
        }
    }
    None
}

fn is_quote_start(node: &Handle) -> bool {
    let NodeData::Element { name, attrs, .. } = &node.data else { return false };
    let attrs = attrs.borrow();
    let local = name.local.to_string();
    let class = attr(&attrs, "class").unwrap_or("").to_ascii_lowercase();
    let id = attr(&attrs, "id").unwrap_or("").to_ascii_lowercase();
    match local.as_str() {
        "blockquote" => attr(&attrs, "type").is_some_and(|t| t.eq_ignore_ascii_case("cite")),
        "div" => {
            if class.split_whitespace().any(|c| c == "gmail_quote" || c == "gmail_quote_container") {
                // A forward is content, not history.
                return !first_text_matching(node, "gmail_attr").is_some_and(|t| t.contains("Forwarded message"));
            }
            if class.contains("yahoo_quoted") || class.contains("moz-cite-prefix") || id == "divrplyfwdmsg" || id == "appendonsend" {
                return true;
            }
            // Outlook desktop: a top border followed by the From:/Sent:/To: header block.
            let style = attr(&attrs, "style").unwrap_or("").to_ascii_lowercase();
            style.contains("border-top") && subtree_text(node, 200).trim_start().starts_with("From:")
        }
        _ => false,
    }
}

fn first_text_matching(node: &Handle, class: &str) -> Option<String> {
    for child in node.children.borrow().iter() {
        if let NodeData::Element { attrs, .. } = &child.data {
            if attr(&attrs.borrow(), "class").is_some_and(|c| c.split_whitespace().any(|x| x == class)) {
                return Some(subtree_text(child, 400));
            }
        }
        if let Some(found) = first_text_matching(child, class) {
            return Some(found);
        }
    }
    None
}

fn subtree_text(node: &Handle, limit: usize) -> String {
    fn walk(node: &Handle, out: &mut String, limit: usize) {
        if out.len() >= limit {
            return;
        }
        match &node.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            _ => {
                for child in node.children.borrow().iter() {
                    walk(child, out, limit);
                }
            }
        }
    }
    let mut out = String::new();
    walk(node, &mut out, limit);
    out
}

fn serialize_children(node: &Handle) -> String {
    let mut buf = Vec::new();
    let opts = SerializeOpts { traversal_scope: TraversalScope::ChildrenOnly(None), ..Default::default() };
    let _ = serialize(&mut buf, &SerializableHandle::from(node.clone()), opts);
    String::from_utf8(buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_what_a_browser_hides() {
        let p = prepare(r#"<html><head><title>Title text</title></head><body><div style="display:none">pre</div><span style="font-size:0px">zero</span><div style="visibility:hidden">vis</div><div class="preheader">ph</div><img src="https://t/x.gif" width="1" height="1"><p>Visible</p></body></html>"#);
        assert!(!p.html.contains("Title text") && !p.html.contains("pre<") && !p.html.contains("zero") && !p.html.contains("vis<") && !p.html.contains("ph<"));
        assert!(!p.html.contains("<img"));
        assert!(p.html.contains("<p>Visible</p>"));
    }

    #[test]
    fn removes_spacer_blocks_and_center() {
        let p = prepare(r#"<div>a</div><div><br></div><p>&nbsp;</p><p><o:p>&nbsp;</o:p></p><center><h1>Head</h1></center><dl><dt>Term</dt><dd>Def</dd></dl>"#);
        assert_eq!(p.html, "<html><body><div>a</div><div><h1>Head</h1></div><dl><div><b>Term</b></div><div>Def</div></dl></body></html>");
    }

    #[test]
    fn links_bare_urls_outside_anchors() {
        let p = prepare(r#"<p>See https://example.com/a?b=1. Also www.example.org, and <a href="https://x.y">https://x.y</a></p>"#);
        assert!(p.html.contains(r#"<a href="https://example.com/a?b=1">https://example.com/a?b=1</a>."#), "{}", p.html);
        assert!(p.html.contains(r#"<a href="https://www.example.org">www.example.org</a>,"#), "{}", p.html);
        assert_eq!(p.html.matches("<a ").count(), 3);
    }

    #[test]
    fn preformatted_lines_survive() {
        let p = prepare("<pre>fn main() {\n    body\n}</pre>");
        assert_eq!(p.html, "<html><body><pre>fn main() {<br>&nbsp;&nbsp;&nbsp;&nbsp;body<br>}</pre></body></html>");
    }

    #[test]
    fn splits_gmail_quote_but_not_forwards() {
        let p = prepare(r#"<div dir="ltr">New text<br><div class="gmail_quote"><div class="gmail_attr">On Mon, X wrote:</div><blockquote class="gmail_quote">old</blockquote></div></div>"#);
        assert!(p.html.contains("New text") && !p.html.contains("old"));
        assert!(p.quoted.as_deref().is_some_and(|q| q.contains("old") && q.contains("wrote:")));
        let f = prepare(r#"<div dir="ltr">FYI<div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------<br>From: A</div><div>body</div></div></div>"#);
        assert!(f.quoted.is_none() && f.html.contains("body"));
    }

    #[test]
    fn splits_cite_blockquotes_and_outlook_headers() {
        let p = prepare(r#"<div>Reply</div><blockquote type="cite"><div>older</div></blockquote>"#);
        assert!(p.quoted.as_deref().is_some_and(|q| q.contains("older")) && !p.html.contains("older"));
        let o = prepare(r#"<div class="WordSection1"><p>Thanks</p><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0in 0in 0in"><p><b>From:</b> A<br><b>Sent:</b> B</p></div><p>original</p></div>"#);
        assert!(o.quoted.as_deref().is_some_and(|q| q.contains("original")) && o.html.contains("Thanks") && !o.html.contains("original"));
    }

    #[test]
    fn trims_breaks_that_only_add_blank_lines() {
        let p = prepare("<div>FYI<br><br><div>next</div><br></div><div><br>lead<br>line<br></div>");
        assert_eq!(p.html, "<html><body><div>FYI<div>next</div></div><div>lead<br>line</div></body></html>");
    }

    #[test]
    fn url_boundaries() {
        assert_eq!(find_url("go to https://a.b/c, then"), Some((6, 13)));
        assert_eq!(find_url("(see www.x.org)"), Some((5, 9)));
        assert_eq!(find_url("mail@www.example.com"), None);
        assert_eq!(find_url("https://x"), None);
    }
}
