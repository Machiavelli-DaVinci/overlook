//! Synthetic email formatting contracts. No snapshots of personal mail or serialized HTML.
use html5ever::{parse_document, tendril::TendrilSink, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use overlook_core::sanitize::{
    adapt_colors, clean_only, html_to_text, sanitize_html, text_to_html, RemoteContent,
};

const OUTLOOK: &str = include_str!("fixtures/formatting/outlook_signature.html");
const GMAIL: &str = include_str!("fixtures/formatting/gmail_div_br.html");
const APPLE: &str = include_str!("fixtures/formatting/apple_inline.html");
const PLAIN: &str = include_str!("fixtures/formatting/plain_quote.txt");
const NEWS: &str = include_str!("fixtures/formatting/newsletter_layout.html");
const DATA: &str = include_str!("fixtures/formatting/data_table.html");
const LISTS: &str = include_str!("fixtures/formatting/nested_lists.html");
const CODE: &str = include_str!("fixtures/formatting/pre_code.html");
const FORWARD: &str = include_str!("fixtures/formatting/forwarded_history.html");
const REPLY: &str = include_str!("fixtures/formatting/reply_history.html");
const UNSAFE: &str = include_str!("fixtures/formatting/unsafe_malformed.html");
const HIDDEN: &str = include_str!("fixtures/formatting/hidden_content.html");
const COLORS: &str = include_str!("fixtures/formatting/color_scope.html");
const CSS: &str = include_str!("fixtures/formatting/css_adversarial.html");

fn corpus() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "outlook",
            OUTLOOK.into(),
            "The synthetic review is complete.",
        ),
        ("gmail", GMAIL.into(), "Second authored line"),
        ("apple", APPLE.into(), "Synthetic café update — ready."),
        (
            "plain",
            text_to_html(PLAIN),
            "Current plain reply <not markup> & confirmation.",
        ),
        ("newsletter", NEWS.into(), "Synthetic field notes"),
        ("data", DATA.into(), "Alpha kit"),
        ("lists", LISTS.into(), "Nested gamma check"),
        ("code", CODE.into(), "fn sample()"),
        ("forward", FORWARD.into(), "Oldest forwarded message"),
        ("reply", REPLY.into(), "Current reply only"),
        ("unsafe", UNSAFE.into(), "Visible recovery end"),
        ("hidden", HIDDEN.into(), "Visible final control"),
        ("colors", COLORS.into(), "Legacy dark ink"),
        ("css", CSS.into(), "Safe trailing declarations"),
    ]
}

fn dom(html: &str) -> RcDom {
    parse_document(RcDom::default(), ParseOpts::default()).one(html)
}

fn walk(node: &Handle, f: &mut impl FnMut(&Handle)) {
    f(node);
    for child in node.children.borrow().iter() {
        walk(child, f);
    }
}

fn text(node: &Handle) -> String {
    let mut result = String::new();
    walk(node, &mut |n| {
        if let NodeData::Text { contents } = &n.data {
            result.push_str(&contents.borrow());
        }
    });
    result
}

// RcDom::drop drains descendants, even if individual handles still have owners.
// Keep the parsed document alive alongside every returned node.
struct Element {
    _dom: std::rc::Rc<RcDom>,
    node: Handle,
}

impl std::ops::Deref for Element {
    type Target = Handle;
    fn deref(&self) -> &Handle {
        &self.node
    }
}

fn elements(html: &str, tag: &str) -> Vec<Element> {
    let parsed = std::rc::Rc::new(dom(html));
    let mut found = Vec::new();
    walk(&parsed.document, &mut |n| {
        if matches!(&n.data, NodeData::Element { name, .. } if &*name.local == tag) {
            found.push(Element {
                _dom: parsed.clone(),
                node: n.clone(),
            });
        }
    });
    found
}

fn attr(node: &Handle, key: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| &*a.name.local == key)
            .map(|a| a.value.to_string()),
        _ => None,
    }
}

// Parsed structural equality, not serializer spelling or attribute-order equality.
fn structure(html: &str) -> Vec<String> {
    fn visit(node: &Handle, out: &mut Vec<String>) {
        match &node.data {
            NodeData::Element { name, attrs, .. } => {
                let mut attrs: Vec<_> = attrs
                    .borrow()
                    .iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect();
                attrs.sort();
                out.push(format!("open:{}:{attrs:?}", name.local));
                for child in node.children.borrow().iter() {
                    visit(child, out);
                }
                out.push(format!("close:{}", name.local));
            }
            NodeData::Text { contents } => out.push(format!("text:{}", contents.borrow())),
            _ => {
                for child in node.children.borrow().iter() {
                    visit(child, out);
                }
            }
        }
    }
    let mut result = Vec::new();
    visit(&dom(html).document, &mut result);
    result
}

fn assert_safe(html: &str, policy: RemoteContent) {
    walk(&dom(html).document, &mut |n| {
        if let NodeData::Element { name, attrs, .. } = &n.data {
            assert!(
                ![
                    "script", "style", "iframe", "object", "embed", "form", "input", "button",
                    "svg", "math", "base", "link", "meta"
                ]
                .contains(&&*name.local),
                "unsafe element: {}",
                name.local
            );
            for a in attrs.borrow().iter() {
                let key = a.name.local.to_string();
                let value = a.value.trim().to_ascii_lowercase();
                assert!(!key.starts_with("on"), "event handler: {key}");
                if key == "href" || key == "src" {
                    assert!(
                        !value.starts_with("javascript:")
                            && !value.starts_with("data:")
                            && !value.starts_with("vbscript:"),
                        "unsafe URL: {value}"
                    );
                }
                if key == "style" {
                    for forbidden in [
                        "url(",
                        "expression(",
                        "behavior:",
                        "position:",
                        "background-image:",
                    ] {
                        assert!(!value.contains(forbidden), "unsafe CSS: {value}");
                    }
                }
                if policy == RemoteContent::Blocked && &*name.local == "img" && key == "src" {
                    assert!(value.starts_with("cid:"), "remote image survived: {value}");
                }
            }
        }
    });
}

fn assert_style(html: &str, tag: &str, visible: &str, property: &str, expected: &str) {
    let node = elements(html, tag)
        .into_iter()
        .find(|n| text(n).contains(visible))
        .unwrap_or_else(|| panic!("missing {tag} containing {visible:?}"));
    let style = attr(&node, "style").unwrap_or_default();
    let value = style
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(property))
        .map(|(_, value)| value.trim().to_ascii_lowercase());
    assert_eq!(
        value.as_deref(),
        Some(expected),
        "{visible:?}: missing/changed {property} in {style:?}"
    );
}

#[test]
fn corpus_is_safe_meaningful_and_cleaning_is_idempotent() {
    for (name, input, marker) in corpus() {
        for policy in [RemoteContent::Blocked, RemoteContent::Allowed] {
            let clean = clean_only(&input, policy);
            assert_safe(&clean, policy);
            assert!(
                html_to_text(&clean).contains(marker),
                "{name}: missing visible text"
            );
            assert_eq!(
                structure(&clean),
                structure(&clean_only(&clean, policy)),
                "{name}: clean_only is not idempotent"
            );
            let reader = sanitize_html(&input, policy);
            assert_safe(&reader.html, policy);
            assert!(
                reader.text.contains(marker),
                "{name}: missing reader/search text"
            );
            if let Some(quoted) = &reader.quoted {
                assert_safe(quoted, policy);
            }
            let second = sanitize_html(&reader.html, policy);
            assert_eq!(
                structure(&reader.html),
                structure(&second.html),
                "{name}: reader body is not structurally stable"
            );
            assert_eq!(
                html_to_text(&reader.html),
                second.text,
                "{name}: reader body text changed"
            );
        }
    }
}

#[test]
fn safe_typography_and_zero_margin_signatures_survive_both_paths() {
    for html in [
        clean_only(OUTLOOK, RemoteContent::Blocked),
        sanitize_html(OUTLOOK, RemoteContent::Blocked).html,
    ] {
        assert_style(&html, "p", "The synthetic review", "margin", "0");
        assert_style(&html, "p", "The synthetic review", "font-size", "11pt");
        assert_style(
            &html,
            "p",
            "The synthetic review",
            "font-family",
            "calibri, sans-serif",
        );
        assert_style(&html, "p", "Example Sender", "font-weight", "700");
        assert_style(&html, "p", "Sample Research Team", "margin-top", "0");
        assert_style(&html, "p", "Sample Research Team", "margin-bottom", "0");
    }
    for html in [
        clean_only(APPLE, RemoteContent::Blocked),
        sanitize_html(APPLE, RemoteContent::Blocked).html,
    ] {
        assert_style(&html, "div", "Synthetic café", "font-size", "16px");
        assert_style(&html, "div", "Synthetic café", "line-height", "1.5");
        // Canonical shorthand retains the same four components (0, 0, 12px, 0).
        assert_style(&html, "div", "Synthetic café", "margin", "0 0 12px");
        assert_style(&html, "span", "Synthetic café", "font-weight", "600");
        assert_style(&html, "span", "Synthetic café", "font-style", "italic");
        assert_style(&html, "span", "Underlined", "text-decoration", "underline");
        assert_style(
            &html,
            "span",
            "Superseded",
            "text-decoration",
            "line-through",
        );
        assert_style(&html, "p", "Centered summary", "text-align", "center");
        assert_style(&html, "p", "Two spaces", "white-space", "pre-wrap");
    }
}

#[test]
fn authored_breaks_and_blank_blocks_are_not_discarded() {
    for input in [GMAIL, OUTLOOK] {
        for html in [
            clean_only(input, RemoteContent::Blocked),
            sanitize_html(input, RemoteContent::Blocked).html,
        ] {
            assert!(
                elements(&html, "br").len() >= elements(input, "br").len(),
                "authored breaks disappeared"
            );
            let blank_blocks = |s: &str| {
                ["p", "div"]
                    .iter()
                    .flat_map(|tag| elements(s, tag))
                    .filter(|n| text(n).trim().is_empty())
                    .count()
            };
            assert!(
                blank_blocks(&html) >= blank_blocks(input),
                "authored spacer blocks disappeared"
            );
            if input == GMAIL {
                let node = elements(&html, "div")
                    .into_iter()
                    .find(|n| text(n).starts_with("After two spacer blocks"))
                    .unwrap();
                let children = node.children.borrow();
                assert!(children.windows(2).any(|pair| pair.iter().all(|n| matches!(&n.data, NodeData::Element { name, .. } if &*name.local == "br"))), "double break was collapsed");
            }
        }
    }
}

#[test]
fn hidden_content_is_removed_even_from_full_history_quotes() {
    // Zero height without clipping does not make overflowing text invisible.
    assert!(clean_only(HIDDEN, RemoteContent::Blocked).contains("VISIBLE_ZERO_HEIGHT_OVERFLOW"));
    assert!(sanitize_html(HIDDEN, RemoteContent::Blocked).text.contains("VISIBLE_ZERO_HEIGHT_OVERFLOW"));
    for input in [HIDDEN, NEWS] {
        for policy in [RemoteContent::Blocked, RemoteContent::Allowed] {
            let reader = sanitize_html(input, policy);
            for output in [
                clean_only(input, policy),
                reader.html,
                reader.quoted.unwrap_or_default(),
            ] {
                assert!(
                    !output.contains("HIDDEN_"),
                    "hidden content leaked into visible/quoted markup: {output}"
                );
            }
        }
    }
}

#[test]
fn adapt_colors_changes_only_color_attributes_not_text_urls_or_typography() {
    for (name, input, _) in corpus() {
        let clean = clean_only(&input, RemoteContent::Allowed);
        for dark in [false, true] {
            let adapted = adapt_colors(&clean, dark);
            assert_eq!(
                text(&dom(&clean).document),
                text(&dom(&adapted).document),
                "{name}: theme changed text"
            );
            assert_eq!(
                html_to_text(&clean),
                html_to_text(&adapted),
                "{name}: theme changed searchable text"
            );
            for tag in ["a", "img", "p", "div", "span", "font", "code"] {
                let before = elements(&clean, tag);
                let after = elements(&adapted, tag);
                assert_eq!(before.len(), after.len(), "{name}: theme changed elements");
                for (a, b) in before.iter().zip(&after) {
                    for key in ["href", "src", "title", "alt", "dir", "lang"] {
                        assert_eq!(attr(a, key), attr(b, key), "{name}: theme changed {key}");
                    }
                    let non_color = |n: &Handle| {
                        attr(n, "style")
                            .unwrap_or_default()
                            .split(';')
                            .filter_map(|d| d.split_once(':'))
                            .filter(|(key, _)| !key.trim().eq_ignore_ascii_case("color"))
                            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        non_color(a),
                        non_color(b),
                        "{name}: theme changed non-color CSS"
                    );
                }
            }
            assert_eq!(
                structure(&adapted),
                structure(&adapt_colors(&adapted, dark)),
                "{name}: theme adaptation is not idempotent"
            );
        }
    }
    let clean = clean_only(COLORS, RemoteContent::Allowed);
    assert_ne!(
        attr(&elements(&clean, "font")[0], "color"),
        attr(&elements(&adapt_colors(&clean, true), "font")[0], "color"),
        "dark legacy ink should actually adapt"
    );
}

#[test]
fn replies_fold_in_reader_but_keep_all_history_when_quoted() {
    for (input, current, previous, oldest) in [(
        REPLY.to_owned(),
        "Current reply only",
        "Previous history message",
        "Oldest history message",
    )] {
        let reader = sanitize_html(&input, RemoteContent::Blocked);
        assert!(html_to_text(&reader.html).contains(current));
        assert!(!html_to_text(&reader.html).contains(previous));
        let quoted = reader
            .quoted
            .as_deref()
            .expect("reply should offer folded history");
        for marker in [previous, oldest] {
            assert!(html_to_text(quoted).contains(marker));
            assert!(
                reader.text.contains(marker),
                "search must retain folded history"
            );
            assert!(
                html_to_text(&clean_only(&input, RemoteContent::Allowed)).contains(marker),
                "outgoing quote lost history"
            );
        }
    }
    // Plain-text quote markers produce generic blockquotes, not client-specific
    // reply wrappers. They retain nesting and history but need not auto-fold.
    let plain_html = text_to_html(PLAIN);
    let plain_reader = sanitize_html(&plain_html, RemoteContent::Blocked);
    assert!(elements(&plain_html, "blockquote").len() >= 2);
    for marker in [
        "Previous sample question",
        "Oldest sample context",
        "Keep the complete history.",
    ] {
        assert!(plain_reader.text.contains(marker));
        assert!(html_to_text(&clean_only(&plain_html, RemoteContent::Allowed)).contains(marker));
    }
    let only_history = sanitize_html(
        "<blockquote type=\"cite\"><p>History without current reply</p></blockquote>",
        RemoteContent::Blocked,
    );
    assert!(only_history.quoted.is_none());
    assert!(html_to_text(&only_history.html).contains("History without current reply"));
}

#[test]
fn forwarded_history_stays_inline_and_complete() {
    let reader = sanitize_html(FORWARD, RemoteContent::Blocked);
    assert!(
        reader.quoted.is_none(),
        "forward must not be mistaken for a reply"
    );
    for marker in [
        "Current forwarding note",
        "Forwarded middle message",
        "Oldest forwarded message",
        "author@example.test",
        "earlier@example.test",
    ] {
        assert!(
            reader.text.contains(marker),
            "missing forward marker: {marker}"
        );
        assert!(html_to_text(&clean_only(FORWARD, RemoteContent::Allowed)).contains(marker));
    }
    assert!(
        !elements(&reader.html, "hr").is_empty(),
        "embedded messages need a separator"
    );
}

#[test]
fn layout_tables_flatten_but_real_data_and_footer_survive() {
    let newsletter = sanitize_html(NEWS, RemoteContent::Blocked);
    assert!(
        elements(&newsletter.html, "table").is_empty(),
        "newsletter layout was retained as a data grid"
    );
    for marker in [
        "Left editorial column",
        "Right editorial column",
        "Read synthetic issue",
    ] {
        assert!(newsletter.text.contains(marker));
    }
    let data = sanitize_html(DATA, RemoteContent::Blocked);
    assert_eq!(elements(&data.html, "table").len(), 1);
    assert_eq!(elements(&data.html, "th").len(), 3);
    assert_eq!(elements(&data.html, "tfoot").len(), 1);
    for marker in [
        "Synthetic allocation",
        "Alpha kit",
        "Beta kit",
        "Total",
        "5",
        "Sample only",
    ] {
        assert!(data.text.contains(marker));
    }
}

#[test]
fn nested_lists_and_preformatted_code_keep_structure_and_lines() {
    let lists = sanitize_html(LISTS, RemoteContent::Blocked);
    assert_eq!(
        elements(&lists.html, "li").len(),
        elements(LISTS, "li").len()
    );
    assert_eq!(
        attr(&elements(&lists.html, "ol")[0], "start").as_deref(),
        Some("3")
    );
    assert!(elements(&lists.html, "li")
        .iter()
        .any(|n| n.children.borrow().iter().any(
            |child| matches!(&child.data, NodeData::Element { name, .. } if &*name.local == "ul")
        )));
    assert!(lists.text.contains("Example definition"));
    let code = sanitize_html(CODE, RemoteContent::Blocked);
    let pre = elements(&code.html, "pre").remove(0);
    let mut breaks = 0;
    walk(&pre, &mut |n| {
        if matches!(&n.data, NodeData::Element { name, .. } if &*name.local == "br") {
            breaks += 1;
        }
    });
    assert!(
        breaks >= 4 || text(&pre).matches('\n').count() >= 4,
        "preformatted line breaks disappeared"
    );
    assert!(
        text(&pre).replace('\u{a0}', " ").contains("    let tag"),
        "preformatted indentation disappeared"
    );
    assert!(code.text.contains("<sample>"));
    assert!(code.text.contains("a < b && b > 0"));
    assert_style(&code.html, "pre", "fn sample", "font-family", "monospace");
    assert_style(&code.html, "pre", "fn sample", "white-space", "pre");
}

#[test]
fn remote_policy_blocks_images_not_links_and_keeps_cid() {
    let blocked = sanitize_html(UNSAFE, RemoteContent::Blocked);
    let allowed = sanitize_html(UNSAFE, RemoteContent::Allowed);
    assert!(blocked.blocked_remote > 0);
    assert_eq!(allowed.blocked_remote, 0);
    assert!(elements(&allowed.html, "img")
        .iter()
        .any(|n| attr(n, "src").as_deref() == Some("https://images.example.test/visible.png")));
    assert!(elements(&blocked.html, "img")
        .iter()
        .any(|n| attr(n, "src").as_deref() == Some("cid:synthetic-logo")));
    for html in [&blocked.html, &allowed.html] {
        assert!(elements(html, "a")
            .iter()
            .any(|n| attr(n, "href").as_deref() == Some("https://example.test/safe?x=1&y=2")));
        for payload in [
            "SCRIPT_PAYLOAD",
            "STYLE_PAYLOAD",
            "FRAME_PAYLOAD",
            "SVG_PAYLOAD",
            "COMMENT_PAYLOAD",
        ] {
            assert!(!html.contains(payload));
        }
    }
}

#[test]
fn css_values_cannot_reintroduce_active_content_or_arbitrary_layout() {
    for html in [
        clean_only(CSS, RemoteContent::Allowed),
        sanitize_html(CSS, RemoteContent::Allowed).html,
    ] {
        assert_safe(&html, RemoteContent::Allowed);
        walk(&dom(&html).document, &mut |n| {
            if let Some(style) = attr(n, "style") {
                let lower = style.to_ascii_lowercase();
                for forbidden in [
                    "expression",
                    "url",
                    "binding",
                    "behavior",
                    "position",
                    "z-index",
                    "content:",
                    "transform",
                ] {
                    assert!(
                        !lower.contains(forbidden),
                        "unsafe/unsupported declaration survived: {style}"
                    );
                }
            }
        });
        assert_style(&html, "p", "Valid mixed-case", "font-size", "14px");
        assert_style(&html, "p", "Safe trailing", "font-style", "italic");
        assert_style(&html, "p", "Safe trailing", "text-align", "right");
        assert_style(&html, "p", "Safe trailing", "margin-left", "8px");
        assert_style(&html, "p", "Safe trailing", "margin-right", "8px");
    }
}

#[test]
fn direct_text_extraction_never_indexes_script_or_style_payloads() {
    let extracted = html_to_text(UNSAFE);
    assert!(extracted.contains("Visible recovery start"));
    assert!(extracted.contains("Visible recovery end"));
    for payload in ["SCRIPT_PAYLOAD", "STYLE_PAYLOAD", "SVG_PAYLOAD"] {
        assert!(
            !extracted.contains(payload),
            "direct html_to_text indexed {payload}"
        );
    }
    let plain = text_to_html(PLAIN);
    assert!(html_to_text(&plain).contains("<not markup> & confirmation."));
    assert!(elements(&plain, "not").is_empty(), "plain text became HTML");
}
