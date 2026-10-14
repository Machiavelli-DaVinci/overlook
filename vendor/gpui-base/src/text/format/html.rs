use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{DefiniteLength, Hsla, SharedString, px, relative};
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, ParseOpts, local_name, parse_document};
use markup5ever_rcdom::{Node, NodeData, RcDom};

use crate::text::document::ParsedDocument;
use crate::text::node::{
    self, BlockNode, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph, Table, TableRow,
    TextMark,
};

mod style;
pub(crate) use style::HtmlParagraphStyle;

const BLOCK_ELEMENTS: [&str; 35] = [
    "html",
    "body",
    "head",
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "summary",
    "dialog",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
    "style",
    "script",
];

/// Parse HTML into AST Node.
pub(crate) fn parse(source: &str, cx: &mut NodeContext) -> Result<ParsedDocument, SharedString> {
    parse_with_context(source, cx, false)
}

/// Markdown's inline HTML is not a complete line box: a br here must keep
/// breaking the surrounding Markdown text, even at the end of this fragment.
pub(crate) fn parse_inline(
    source: &str,
    cx: &mut NodeContext,
) -> Result<ParsedDocument, SharedString> {
    parse_with_context(source, cx, true)
}

fn parse_with_context(
    source: &str,
    cx: &mut NodeContext,
    inline: bool,
) -> Result<ParsedDocument, SharedString> {
    let opts = ParseOpts {
        ..Default::default()
    };

    // Collapse HTML whitespace in the AST, not in a minifier: authored pre-wrap
    // must survive and whitespace runs can span inline element boundaries.
    let bytes = source.as_bytes();
    let mut cursor = std::io::Cursor::new(bytes);
    // Ref
    // https://github.com/servo/html5ever/blob/main/rcdom/examples/print-rcdom.rs
    let dom = parse_document(RcDom::default(), opts)
        .from_utf8()
        .read_from(&mut cursor)
        .map_err(|e| SharedString::from(format!("{:?}", e)))?;

    let mut paragraph = Paragraph::default();
    // NOTE: The outer paragraph is not used.
    let node: BlockNode =
        parse_node(&dom.document, &mut paragraph, cx).unwrap_or(BlockNode::Unknown);
    let mut node = node.compact();
    if !inline {
        close_html_line_boxes(&mut node);
    }

    Ok(ParsedDocument {
        source: source.to_string().into(),
        blocks: Arc::new(vec![node]),
    })
}

/// A terminal HTML br closes its current line, without creating another empty
/// line box. A break-only block still has one line; two brs have two. Keep
/// Markdown fragments and preserved literal whitespace out of this adjustment.
fn close_html_line_boxes(block: &mut BlockNode) {
    fn close(paragraph: &mut Paragraph) {
        if paragraph.html_style.as_ref().is_some_and(|s| s.preserve) {
            return;
        }
        let Some(ix) = paragraph
            .children
            .iter()
            .rposition(|n| !n.text.is_empty() || n.image.is_some())
        else {
            return;
        };
        let node = &paragraph.children[ix];
        if node.image.is_some() || !node.text.ends_with('\n') {
            return;
        }
        let len = node.text.len() - 1;
        // push_str gives br its own inline node. Only supply a blank-line
        // strut when there is no preceding content on that line; otherwise
        // x<br> would acquire a phantom space (and an image could wrap).
        let mut line_has_content = false;
        for previous in paragraph.children[..ix].iter().rev() {
            if previous.image.is_some() {
                line_has_content = true;
                break;
            }
            let tail = previous.text.rsplit('\n').next().unwrap_or("");
            if tail.chars().any(|ch| ch != ' ') {
                line_has_content = true;
                break;
            }
            if previous.text.contains('\n') {
                break;
            }
        }
        let blank_line = len == 0 && !line_has_content;
        let node = &mut paragraph.children[ix];
        node.text = if blank_line {
            "\u{a0}".into()
        } else {
            node.text[..len].to_owned().into()
        };
        for (range, _) in &mut node.marks {
            if blank_line && range.start < range.end {
                *range = 0..node.text.len();
            } else {
                range.start = range.start.min(len);
                range.end = range.end.min(len);
            }
        }
    }
    match block {
        BlockNode::Paragraph(p) | BlockNode::Heading { children: p, .. } => close(p),
        BlockNode::Root { children, .. }
        | BlockNode::Blockquote { children, .. }
        | BlockNode::List { children, .. }
        | BlockNode::ListItem { children, .. } => {
            for child in children {
                close_html_line_boxes(child);
            }
        }
        BlockNode::Table(table) => {
            for row in &mut table.children {
                for cell in &mut row.children {
                    close(&mut cell.children);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
fn cleanup_html(source: &str) -> Vec<u8> {
    let mut w = std::io::Cursor::new(vec![]);
    let mut r = std::io::Cursor::new(source);
    let mut minify = super::html5minify::Minifier::new(&mut w);
    minify.omit_doctype(true);
    if let Ok(()) = minify.minify(&mut r) {
        w.into_inner()
    } else {
        source.bytes().collect()
    }
}

fn attr_value(attrs: &RefCell<Vec<html5ever::Attribute>>, name: LocalName) -> Option<String> {
    attrs.borrow().iter().find_map(|attr| {
        if attr.name.local == name {
            Some(attr.value.to_string())
        } else {
            None
        }
    })
}

/// Default `<mark>` background when the tag has no usable color.
pub(super) fn default_mark_color() -> Hsla {
    gpui::rgb(0xfef08a).into()
}

/// Get the highlight background color for a `<mark>` element.
///
/// Reads the `color` attribute first, then the `background-color` declaration
/// from the `style` attribute. Base accepts CSS hex plus common named colors.
fn mark_color(attrs: &RefCell<Vec<html5ever::Attribute>>) -> Option<Hsla> {
    mark_color_from_values(
        attr_value(attrs, local_name!("color")).as_deref(),
        attr_value(attrs, local_name!("style")).as_deref(),
    )
}

/// Resolve a `<mark>` color from raw `color` / `style` attribute values.
///
/// Same rules as the block HTML path: `color` wins, then
/// `style="background-color: …"`.
pub(super) fn mark_color_from_values(color: Option<&str>, style: Option<&str>) -> Option<Hsla> {
    if let Some(value) = color
        && let Some(color) = parse_mark_color(value.trim())
    {
        return Some(color);
    }

    style.and_then(|css_text| parse_mark_color(style_attrs(css_text).get("background-color")?))
}

/// Overlook patch: CSS colours as mail uses them: `#rgb`, `#rrggbb`, `rgb(r, g, b)` and the
/// common names.
pub(super) fn parse_css_color(value: &str) -> Option<Hsla> {
    let v = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let hex = hex.trim();
        let expanded: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_owned()
        };
        if expanded.len() != 6 {
            return None;
        }
        return u32::from_str_radix(&expanded, 16)
            .ok()
            .map(|n| gpui::rgb(n).into());
    }
    if let Some(inner) = v.strip_prefix("rgb(").or_else(|| v.strip_prefix("rgba(")) {
        let inner = inner.strip_suffix(')')?;
        let parts: Vec<_> = inner.split(',').map(str::trim).collect();
        let has_alpha = v.starts_with("rgba(");
        if parts.len() != if has_alpha { 4 } else { 3 } {
            return None;
        }
        let channel = |value: &str| -> Option<u32> {
            let (number, factor) = value
                .strip_suffix('%')
                .map(|n| (n, 2.55))
                .unwrap_or((value, 1.));
            let number = number.parse::<f32>().ok()?;
            number
                .is_finite()
                .then(|| (number * factor).clamp(0., 255.).round() as u32)
        };
        let (r, g, b) = (channel(parts[0])?, channel(parts[1])?, channel(parts[2])?);
        let alpha = if has_alpha {
            parts[3].parse::<f32>().ok()?
        } else {
            1.
        };
        if !alpha.is_finite() {
            return None;
        }
        return Some(
            gpui::rgb((r << 16) | (g << 8) | b)
                .alpha(alpha.clamp(0., 1.))
                .into(),
        );
    }
    let (r, g, b) = if v == "windowtext" {
        (0, 0, 0)
    } else {
        cssparser::color::parse_named_color(&v).ok()?
    };
    Some(gpui::rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into())
}

fn parse_mark_color(value: &str) -> Option<Hsla> {
    if value.starts_with('#') {
        return gpui::Rgba::try_from(value).ok().map(Into::into);
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => Some(gpui::rgb(0x000000).into()),
        "white" => Some(gpui::rgb(0xffffff).into()),
        "blue" => Some(gpui::rgb(0x3b82f6).into()),
        "yellow" => Some(gpui::rgb(0xfacc15).into()),
        _ => None,
    }
}

/// Get style properties to HashMap
/// TODO: Use cssparser to parse style attribute.
fn style_attrs(css_text: &str) -> HashMap<String, String> {
    let mut styles = HashMap::new();
    for decl in css_text.split(';') {
        let mut parts = decl.splitn(2, ':');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            styles.insert(
                key.trim().to_lowercase().to_string(),
                value.trim().to_string(),
            );
        }
    }

    styles
}

/// Parse length value from style attribute.
///
/// When is percentage, it will be converted to relative length.
/// Else, it will be converted to pixels.
fn value_to_length(value: &str) -> Option<DefiniteLength> {
    if value.ends_with("%") {
        value
            .trim_end_matches("%")
            .parse::<f32>()
            .ok()
            .map(|v| relative(v / 100.))
    } else {
        value
            .trim_end_matches("px")
            .parse()
            .ok()
            .map(|v| px(v).into())
    }
}

/// Get width, height from attributes or parse them from style attribute.
fn attr_width_height(
    attrs: &RefCell<Vec<html5ever::Attribute>>,
) -> (Option<DefiniteLength>, Option<DefiniteLength>) {
    let mut width = None;
    let mut height = None;

    if let Some(value) = attr_value(attrs, local_name!("width")) {
        width = value_to_length(&value);
    }

    if let Some(value) = attr_value(attrs, local_name!("height")) {
        height = value_to_length(&value);
    }

    if width.is_none() || height.is_none() {
        let styles = attr_value(attrs, local_name!("style"))
            .map(|css_text| style_attrs(&css_text))
            .unwrap_or_default();
        if width.is_none() {
            width = styles.get("width").and_then(|v| value_to_length(&v));
        }
        if height.is_none() {
            height = styles.get("height").and_then(|v| value_to_length(&v));
        }
    }

    (width, height)
}

fn parse_table_row(table: &mut Table, node: &Rc<Node>) {
    let mut row = TableRow::default();
    let mut count = 0;
    for child in node.children.borrow().iter() {
        match child.data {
            NodeData::Element {
                ref name,
                ref attrs,
                ..
            } if name.local == local_name!("td") || name.local == local_name!("th") => {
                // Overlook patch: an empty cell still occupies its column.
                count += 1;
                parse_table_cell(&mut row, child, attrs);
            }
            _ => {}
        }
    }

    if count > 0 {
        let header = node.children.borrow().iter().any(|child| {
            matches!(&child.data, NodeData::Element { name, .. } if name.local == local_name!("th"))
        });
        if let Some(headers) = &mut table.html_headers {
            headers.push(header);
        }
        table.children.push(row);
    }
}

fn parse_table_cell(
    row: &mut node::TableRow,
    node: &Rc<Node>,
    attrs: &RefCell<Vec<html5ever::Attribute>>,
) {
    let mut paragraph = Paragraph {
        html_style: Some(style::resolve(node).paragraph),
        ..Default::default()
    };
    for child in node.children.borrow().iter() {
        if is_block_child(child) && !paragraph.is_empty() {
            paragraph.push_str("\n");
        }
        parse_paragraph(&mut paragraph, child);
    }
    normalize_whitespace(&mut paragraph);
    let width = attr_width_height(attrs).0;
    let table_cell = node::TableCell {
        children: paragraph,
        width,
    };
    row.children.push(table_cell);
    // Overlook patch: a spanning cell is followed by empty cells so the columns after it
    // stay aligned (the content does not stretch, but nothing shifts left).
    let span = attr_value(attrs, local_name!("colspan"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1);
    for _ in 1..span.min(16) {
        row.children.push(node::TableCell {
            children: Paragraph::default(),
            width: None,
        });
    }
}

/// Trim text but leave at least one space.
///
/// - Before: " \r\n Hello world \t "
/// - After: " Hello world "
#[allow(dead_code)]
fn trim_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for (i, c) in text.chars().enumerate() {
        if c.is_whitespace() {
            if i > 0 && out.ends_with(' ') {
                continue;
            }
        }
        out.push(c);
    }

    out
}

fn parse_paragraph(paragraph: &mut Paragraph, node: &Rc<Node>) {
    paragraph
        .html_style
        .get_or_insert_with(|| style::resolve(node).paragraph);
    fn push_merged(
        paragraph: &mut Paragraph,
        text: String,
        marks: Vec<(Range<usize>, TextMark)>,
        new_mark: Option<TextMark>,
    ) {
        if text.is_empty() {
            return;
        }
        let mut node = InlineNode::new(text).marks(marks);
        if let Some(new_mark) = new_mark {
            let len = node.text.len();
            let start = len - node.text.trim_start_matches(' ').len();
            let end = node.text.trim_end_matches(' ').len().max(start);
            if let Some(last) = node.marks.last_mut()
                && start == 0
                && end == len
                && last.0.start == 0
                && last.0.end == len
            {
                let child = last.1.clone();
                last.1 = new_mark;
                last.1.merge(child);
            } else {
                // Ancestor marks are the base layer; child declarations win.
                node.marks.insert(0, (start..end, new_mark));
            }
        }
        paragraph.push(node);
    }

    fn merge_children_with_mark(
        node: &Node,
        paragraph: &mut Paragraph,
        new_mark: Option<TextMark>,
    ) {
        let mut merged_text = String::new();
        let mut merged_marks = Vec::new();

        let mut child_paragraph = Paragraph::default();
        for child in node.children.borrow().iter() {
            parse_paragraph(&mut child_paragraph, child);
        }
        if child_paragraph
            .html_style
            .as_ref()
            .is_some_and(|s| s.preserve)
        {
            if let Some(style) = &mut paragraph.html_style {
                style.preserve = true;
            }
        }
        {
            for node in child_paragraph.children {
                let offset = merged_text.len();
                merged_text.push_str(&node.text);
                for (range, child_mark) in node.marks {
                    merged_marks.push((range.start + offset..range.end + offset, child_mark));
                }

                if let Some(mut image) = node.image {
                    if let Some(link_mark) = new_mark.as_ref().and_then(|mark| mark.link.clone()) {
                        image.link = Some(link_mark);
                    }

                    push_merged(
                        paragraph,
                        std::mem::take(&mut merged_text),
                        std::mem::take(&mut merged_marks),
                        new_mark.clone(),
                    );

                    paragraph.push(InlineNode::image(image));
                }
            }
        }

        push_merged(paragraph, merged_text, merged_marks, new_mark.clone());
    }

    match &node.data {
        NodeData::Text { contents } => {
            let part = &contents.borrow();
            style::push_text(paragraph, node, &part);
        }
        NodeData::Element { name, attrs, .. } => match name.local {
            // Overlook patch: a <br> inside inline content is a hard line break, like the
            // Markdown hard-break path. Upstream dropped it, which ran lines together.
            local_name!("br") => {
                paragraph.push_str("\n");
            }
            local_name!("em") | local_name!("i") => {
                merge_children_with_mark(node, paragraph, Some(TextMark::default().italic()));
            }
            local_name!("strong") | local_name!("b") => {
                merge_children_with_mark(node, paragraph, Some(TextMark::default().bold()));
            }
            local_name!("del") | local_name!("s") => {
                merge_children_with_mark(
                    node,
                    paragraph,
                    Some(TextMark::default().strikethrough()),
                );
            }
            local_name!("u") => {
                merge_children_with_mark(node, paragraph, Some(TextMark::default().underline()));
            }
            local_name!("code") => {
                merge_children_with_mark(node, paragraph, Some(TextMark::default().code()));
            }
            local_name!("mark") => {
                let color = mark_color(&attrs).unwrap_or_else(default_mark_color);
                merge_children_with_mark(
                    node,
                    paragraph,
                    Some(TextMark::default().highlight(color)),
                );
            }
            // Overlook patch: an author's text colour (`<span style="color:red">`,
            // `<font color="#c00">`) is kept; it carries meaning in annotated replies.
            local_name!("span") | local_name!("font") => {
                let color = attr_value(&attrs, local_name!("color"))
                    .as_deref()
                    .and_then(parse_css_color)
                    .or_else(|| {
                        attr_value(&attrs, local_name!("style")).and_then(|css| {
                            style_attrs(&css)
                                .get("color")
                                .and_then(|v| parse_css_color(v))
                        })
                    });
                merge_children_with_mark(
                    node,
                    paragraph,
                    color.map(|c| TextMark::default().color(c)),
                );
            }
            local_name!("a") => {
                let link_mark = LinkMark {
                    url: attr_value(&attrs, local_name!("href"))
                        .unwrap_or_default()
                        .into(),
                    title: attr_value(&attrs, local_name!("title")).map(Into::into),
                    ..Default::default()
                };

                merge_children_with_mark(
                    node,
                    paragraph,
                    Some(TextMark::default().link(link_mark)),
                );
            }
            local_name!("img") => {
                let Some(src) = attr_value(attrs, local_name!("src")) else {
                    if cfg!(debug_assertions) {
                        tracing::warn!("Image node missing src attribute");
                    }
                    return;
                };

                let alt = attr_value(attrs, local_name!("alt"));
                let title = attr_value(attrs, local_name!("title"));
                let (width, height) = attr_width_height(attrs);

                paragraph.push_image(ImageNode {
                    url: src.into(),
                    alt: alt.map(Into::into),
                    width,
                    height,
                    title: title.map(Into::into),
                    ..Default::default()
                });
            }
            _ => {
                merge_children_with_mark(node, paragraph, None);
            }
        },
        _ => {
            merge_children_with_mark(node, paragraph, None);
        }
    }
}

fn parse_node(
    node: &Rc<Node>,
    paragraph: &mut Paragraph,
    cx: &mut NodeContext,
) -> Option<BlockNode> {
    match node.data {
        NodeData::Text { ref contents } => {
            style::push_text(paragraph, node, &contents.borrow());

            None
        }
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => match name.local {
            // Overlook patch: <br> between block children breaks the line, not the block,
            // so "a<br>b" inside a <div> keeps Gmail's tight line spacing.
            local_name!("br") => {
                paragraph.push_str("\n");
                None
            }
            local_name!("h1")
            | local_name!("h2")
            | local_name!("h3")
            | local_name!("h4")
            | local_name!("h5")
            | local_name!("h6") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);

                let level = name
                    .local
                    .chars()
                    .last()
                    .unwrap_or('6')
                    .to_digit(10)
                    .unwrap_or(6) as u8;

                let mut paragraph = Paragraph::default();
                for child in node.children.borrow().iter() {
                    parse_paragraph(&mut paragraph, child);
                }

                let heading = BlockNode::Heading {
                    level,
                    children: paragraph,
                    span: None,
                };
                if children.len() > 0 {
                    children.push(heading);

                    Some(BlockNode::Root {
                        children,
                        span: None,
                    })
                } else {
                    Some(heading)
                }
            }
            local_name!("img") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);

                let Some(src) = attr_value(attrs, local_name!("src")) else {
                    if cfg!(debug_assertions) {
                        tracing::warn!("image node missing src attribute");
                    }
                    return None;
                };

                let alt = attr_value(&attrs, local_name!("alt"));
                let title = attr_value(&attrs, local_name!("title"));
                let (width, height) = attr_width_height(&attrs);

                let mut paragraph = Paragraph::default();
                paragraph.push_image(ImageNode {
                    url: src.into(),
                    title: title.map(Into::into),
                    alt: alt.map(Into::into),
                    width,
                    height,
                    ..Default::default()
                });

                if children.len() > 0 {
                    children.push(BlockNode::Paragraph(paragraph));
                    Some(BlockNode::Root {
                        children,
                        span: None,
                    })
                } else {
                    Some(BlockNode::Paragraph(paragraph))
                }
            }
            local_name!("ul") | local_name!("ol") => {
                let ordered = name.local == local_name!("ol");
                // Overlook patch: honour <ol start="n">.
                let start = if ordered {
                    attrs
                        .borrow()
                        .iter()
                        .find(|a| &*a.name.local == "start")
                        .and_then(|a| a.value.trim().parse::<u32>().ok())
                } else {
                    None
                };
                let mut before = vec![];
                consume_paragraph(&mut before, paragraph);
                let children = consume_children_nodes(node, paragraph, cx);
                let list = BlockNode::List {
                    children,
                    ordered,
                    start,
                    span: None,
                };
                if before.is_empty() {
                    Some(list)
                } else {
                    before.push(list);
                    Some(BlockNode::Root {
                        children: before,
                        span: None,
                    })
                }
            }
            local_name!("li") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);

                children.extend(consume_children_nodes(node, paragraph, cx));

                consume_paragraph(&mut children, paragraph);

                Some(BlockNode::ListItem {
                    children,
                    spread: false,
                    checked: None,
                    span: None,
                })
            }
            local_name!("table") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);

                let mut table = Table {
                    html_headers: Some(vec![]),
                    ..Default::default()
                };
                for child in node.children.borrow().iter() {
                    match child.data {
                        NodeData::Element { ref name, .. }
                            if name.local == local_name!("tbody")
                                || name.local == local_name!("thead")
                                || name.local == local_name!("tfoot") =>
                        {
                            for sub_child in child.children.borrow().iter() {
                                parse_table_row(&mut table, &sub_child);
                            }
                        }
                        _ => {
                            parse_table_row(&mut table, &child);
                        }
                    }
                }
                consume_paragraph(&mut children, paragraph);

                let table = BlockNode::Table(table);
                if children.len() > 0 {
                    children.push(table);
                    Some(BlockNode::Root {
                        children,
                        span: None,
                    })
                } else {
                    Some(table)
                }
            }
            local_name!("blockquote") => {
                let mut before = vec![];
                consume_paragraph(&mut before, paragraph);
                let children = consume_children_nodes(node, paragraph, cx);
                let quote = BlockNode::Blockquote {
                    children,
                    span: None,
                };
                if before.is_empty() {
                    Some(quote)
                } else {
                    before.push(quote);
                    Some(BlockNode::Root {
                        children: before,
                        span: None,
                    })
                }
            }
            local_name!("style") | local_name!("script") => None,
            // Overlook patch: <hr> is a rule, not an empty block.
            local_name!("hr") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);
                let rule = BlockNode::HorizontalRule { span: None };
                if children.is_empty() {
                    Some(rule)
                } else {
                    children.push(rule);
                    Some(BlockNode::Root {
                        children,
                        span: None,
                    })
                }
            }
            // Overlook patch: <pre> keeps its line structure and renders as a code block.
            local_name!("pre") => {
                let mut children = vec![];
                consume_paragraph(&mut children, paragraph);
                let mut code = String::new();
                collect_preformatted(node, &mut code);
                let code = code.trim_end_matches('\n').to_string();
                let block = BlockNode::CodeBlock(crate::text::node::CodeBlock::new(
                    SharedString::from(code),
                    None,
                    None::<crate::text::node::Span>,
                ));
                if children.is_empty() {
                    Some(block)
                } else {
                    children.push(block);
                    Some(BlockNode::Root {
                        children,
                        span: None,
                    })
                }
            }
            _ => {
                if BLOCK_ELEMENTS.contains(&name.local.trim()) {
                    let mut children: Vec<BlockNode> = vec![];

                    // Case:
                    //
                    // Hello <p>Inner text of block element</p> World

                    // Insert before text as a node -- The "Hello"
                    consume_paragraph(&mut children, paragraph);
                    paragraph.html_style = Some(style::resolve(node).paragraph);

                    // Inner of the block element -- The "Inner text of block element"
                    for child in node.children.borrow().iter() {
                        if let Some(child_node) = parse_node(child, paragraph, cx) {
                            children.push(child_node);
                        }
                    }
                    consume_paragraph(&mut children, paragraph);
                    paragraph.html_style = None;

                    if children.is_empty() {
                        None
                    } else {
                        Some(BlockNode::Root {
                            children,
                            span: None,
                        })
                    }
                } else {
                    // Others to as Inline
                    parse_paragraph(paragraph, node);

                    if paragraph.is_image() {
                        Some(BlockNode::Paragraph(paragraph.take()))
                    } else {
                        None
                    }
                }
            }
        },
        NodeData::Document => {
            let children = consume_children_nodes(node, paragraph, cx);
            Some(BlockNode::Root {
                children,
                span: None,
            })
        }
        NodeData::Doctype { .. }
        | NodeData::Comment { .. }
        | NodeData::ProcessingInstruction { .. } => None,
    }
}

fn consume_children_nodes(
    node: &Rc<Node>,
    paragraph: &mut Paragraph,
    cx: &mut NodeContext,
) -> Vec<BlockNode> {
    let mut children = vec![];
    consume_paragraph(&mut children, paragraph);
    paragraph.html_style = Some(style::resolve(node).paragraph);
    for child in node.children.borrow().iter() {
        // Overlook patch: flush the running text only before a block child, so inline
        // runs and <br> line breaks inside a blockquote or list item stay one paragraph.
        if is_block_child(child) {
            consume_paragraph(&mut children, paragraph);
        }
        if let Some(child_node) = parse_node(child, paragraph, cx) {
            children.push(child_node);
        }
    }
    consume_paragraph(&mut children, paragraph);
    paragraph.html_style = None;

    children
}

fn is_block_child(node: &Rc<Node>) -> bool {
    match &node.data {
        NodeData::Element { name, .. } => {
            let local: &str = &name.local;
            local == "li" || BLOCK_ELEMENTS.contains(&local)
        }
        _ => false,
    }
}

/// Overlook patch: the text of a `<pre>` with `<br>` as newlines and non-breaking spaces
/// (which survive HTML whitespace collapsing) turned back into spaces.
fn collect_preformatted(node: &Rc<Node>, out: &mut String) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow().replace('\u{a0}', " ")),
            NodeData::Element { name, .. } => {
                if name.local == local_name!("br") {
                    out.push('\n');
                } else {
                    collect_preformatted(child, out);
                }
            }
            _ => {}
        }
    }
}

fn consume_paragraph(children: &mut Vec<BlockNode>, paragraph: &mut Paragraph) {
    normalize_whitespace(paragraph);
    if paragraph.is_empty() {
        return;
    }

    children.push(BlockNode::Paragraph(paragraph.take()));
}

/// Collapse across element boundaries while remapping UTF-8 mark offsets.
fn normalize_whitespace(paragraph: &mut Paragraph) {
    if paragraph.html_style.as_ref().is_some_and(|s| s.preserve) {
        return;
    }
    let mut space = true;
    for node in &mut paragraph.children {
        if node.image.is_some() {
            space = false;
            continue;
        }
        let mut offsets = vec![0; node.text.len() + 1];
        let mut text = String::new();
        for (ix, ch) in node.text.char_indices() {
            offsets[ix] = text.len();
            if ch != ' ' || !space {
                text.push(ch);
            }
            space = ch == ' ' || ch == '\n';
            offsets[ix + ch.len_utf8()] = text.len();
        }
        for (range, _) in &mut node.marks {
            range.start = offsets[range.start.min(node.text.len())];
            range.end = offsets[range.end.min(node.text.len())];
        }
        node.text = text.into();
    }
    if let Some(node) = paragraph
        .children
        .iter_mut()
        .rev()
        .find(|n| !n.text.is_empty() || n.image.is_some())
    {
        if node.image.is_none() {
            let len = node.text.trim_end_matches(' ').len();
            node.text = node.text[..len].to_owned().into();
            for (range, _) in &mut node.marks {
                range.end = range.end.min(len);
                range.start = range.start.min(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{px, relative};
    use std::sync::Arc;

    use crate::text::{
        document::ParsedDocument,
        node::{BlockNode, ImageNode, InlineNode, NodeContext, Paragraph},
    };

    use super::trim_text;

    fn paragraphs<'a>(node: &'a BlockNode, out: &mut Vec<&'a Paragraph>) {
        match node {
            BlockNode::Paragraph(p) => out.push(p),
            BlockNode::Heading { children, .. } => out.push(children),
            BlockNode::Root { children, .. }
            | BlockNode::List { children, .. }
            | BlockNode::ListItem { children, .. }
            | BlockNode::Blockquote { children, .. } => {
                for child in children {
                    paragraphs(child, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn html_paragraphs_have_distinct_bounded_spacing_and_inherit_typography() {
        let document = super::parse(r#"<div style="font-size:15pt;line-height:1.8;color:red;text-align:right"><div>a</div><p style="margin:2px 3px 4px;margin-top:2000px">b</p><div style="font-size:150%;margin-bottom:-10px">c</div></div>"#, &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        assert_eq!(ps.len(), 3);
        let styles: Vec<_> = ps.iter().map(|p| p.html_style.as_ref().unwrap()).collect();
        assert_eq!((styles[0].margin_top, styles[0].margin_bottom), (0., 0.));
        assert_eq!((styles[1].margin_top, styles[1].margin_bottom), (96., 4.));
        assert_eq!(styles[0].font_size, 20.);
        assert_eq!(styles[0].line_height, 1.8);
        assert_eq!(styles[0].align, "right");
        assert_eq!(styles[2].font_size, 30.);
        assert_eq!(styles[2].margin_bottom, 0.);
        assert_eq!(
            ps[0].children[0].marks.last().unwrap().1.color,
            super::parse_css_color("red")
        );
    }

    #[test]
    fn authored_units_font_attributes_and_decorations() {
        let document = super::parse(r##"<div style="font-size:20px;line-height:24pt"><p style="font-size:150%;margin-top:2em;margin:1rem 0 50%">x</p><div><font face="Example" size="7" color="#123">y</font><span style="text-decoration:underline line-through">z</span></div></div>"##, &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        let style = ps[0].html_style.as_ref().unwrap();
        assert_eq!(style.font_size, 30.);
        assert_eq!(style.margin_top, 16.);
        assert!((style.margin_bottom - 15.).abs() < 0.01);
        assert!((style.line_height * style.font_size - 32.).abs() < 0.01);
        let y = &ps[1].children[0].marks.last().unwrap().1;
        assert_eq!(y.font_family.as_deref(), Some("Example"));
        assert_eq!(y.font_size_scale, Some(48. / 20.));
        assert_eq!(y.color, super::parse_css_color("#123"));
        let z = ps[1].children[1].marks.last().unwrap();
        assert!(z.1.underline && z.1.strikethrough);
        let document = super::parse(
            "<p style='font-size:10000rem;line-height:NaN;margin:infpx'>x</p>",
            &mut NodeContext::default(),
        )
        .unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        assert_eq!(ps[0].html_style.as_ref().unwrap().font_size, 64.);
        assert!(ps[0].html_style.as_ref().unwrap().margin_top.is_finite());
    }

    #[test]
    fn nested_inline_typography_child_wins() {
        let document = super::parse(r#"<p><b style="color:red"><span style="color:blue;font-size:200%;font-weight:normal;font-style:italic;font-family:Example">child</span></b></p>"#, &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        let mark = &ps[0].children[0].marks.last().unwrap().1;
        assert_eq!(mark.color, super::parse_css_color("blue"));
        assert_eq!(mark.font_size_scale, Some(2.));
        assert_eq!(mark.font_weight, Some(gpui::FontWeight::NORMAL));
        assert_eq!(mark.font_style, Some(gpui::FontStyle::Italic));
        assert_eq!(mark.font_family.as_deref(), Some("Example"));
    }

    #[test]
    fn terminal_break_closes_content_without_inserting_a_space() {
        for source in ["<div>x<br></div>", "<div><span>é中</span><br></div>"] {
            let doc = super::parse(source, &mut NodeContext::default()).unwrap();
            let expected = if source.contains('é') {
                "é中\n"
            } else {
                "x\n"
            };
            assert_eq!(doc.blocks[0].text(), expected);
        }
        let doc = super::parse(
            "<div><span>x<img src='https://example.test/image.png'><br></span></div>",
            &mut NodeContext::default(),
        )
        .unwrap();
        let mut ps = vec![];
        paragraphs(&doc.blocks[0], &mut ps);
        assert!(
            ps.iter()
                .flat_map(|p| &p.children)
                .any(|n| n.image.is_some())
        );
        assert!(
            ps.iter()
                .flat_map(|p| &p.children)
                .all(|n| !n.text.contains('\u{a0}'))
        );
    }

    #[test]
    fn whitespace_breaks_and_list_block_boundaries() {
        let document = super::parse("<div> a <span> b </span> c<br>d </div><ul><li>before<p>inside</p>after<br>end<ul><li>nested</li></ul>tail</li></ul><div>last</div>", &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        let texts: Vec<_> = ps.iter().map(|p| p.text().to_string()).collect();
        assert_eq!(
            texts,
            [
                "a b c\nd",
                "before",
                "inside",
                "after\nend",
                "nested",
                "tail",
                "last"
            ]
        );
        let document = super::parse(
            "<div style='white-space:pre-wrap'> a  b\n c</div>",
            &mut NodeContext::default(),
        )
        .unwrap();
        let mut ps = vec![];
        paragraphs(&document.blocks[0], &mut ps);
        assert_eq!(ps[0].text(), " a  b\n c");
    }

    #[test]
    fn html_table_headers_are_explicit_and_footer_survives() {
        let document = super::parse("<table><tbody><tr><td>body</td></tr><tr><th>header</th></tr></tbody><tfoot><tr><td>footer</td></tr></tfoot></table>", &mut NodeContext::default()).unwrap();
        let BlockNode::Table(table) = &document.blocks[0] else {
            panic!("expected table")
        };
        assert_eq!(table.html_headers, Some(vec![false, true, false]));
        assert_eq!(table.children.len(), 3);
        assert_eq!(table.table_data().headers, ["header"]);
        assert_eq!(table.table_data().rows, [vec!["body"], vec!["footer"]]);
    }

    #[test]
    fn horizontal_spacing_relative_fonts_and_full_color_policy() {
        let doc = super::parse("<p align='right' style='margin:1px 2px 3px 4px;margin-left:6px'><font size='+1'>up</font><font size='-1'>down</font><small>small</small></p>", &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&doc.blocks[0], &mut ps);
        let style = ps[0].html_style.as_ref().unwrap();
        assert_eq!(
            (
                style.margin_top,
                style.margin_right,
                style.margin_bottom,
                style.margin_left
            ),
            (1., 2., 3., 6.)
        );
        assert_eq!(style.align, "right");
        let reset = super::parse("<div style='line-height:40px;text-align:center'><p style='line-height:normal;text-align:left'>reset</p></div>", &mut NodeContext::default()).unwrap();
        let mut resets = vec![];
        paragraphs(&reset.blocks[0], &mut resets);
        let reset_style = resets[0].html_style.as_ref().unwrap();
        assert_eq!(reset_style.line_height, 1.4);
        assert_eq!(reset_style.align, "left");
        for (text, scale) in [("up", 18. / 16.), ("down", 13. / 16.), ("small", 0.8333333)] {
            let node = ps[0]
                .children
                .iter()
                .find(|n| n.text.as_ref() == text)
                .unwrap();
            let actual = node
                .marks
                .iter()
                .find_map(|(_, m)| m.font_size_scale)
                .unwrap();
            assert!((actual - scale).abs() < 0.001);
        }
        let rgba = gpui::Rgba::from(super::parse_css_color("rgba(100%,0%,50%,0.25)").unwrap());
        assert!((rgba.r - 1.).abs() < 0.001 && (rgba.b - 128. / 255.).abs() < 0.001);
        assert!((rgba.a - 0.25).abs() < 0.001);
        assert!(super::parse_css_color("rebeccapurple").is_some());
        assert!(super::parse_css_color("rgb(1,2,3,4)").is_none());
    }

    #[test]
    fn html_semantic_defaults_override_inheritance_but_not_explicit_css() {
        let doc = super::parse("<div style='font-family:Example Sans;font-weight:normal'><h1>Heading</h1><h1 style='font-weight:normal'>Plain heading</h1><p><code>code</code><code style='font-family:Example Sans'>styled</code></p></div>", &mut NodeContext::default()).unwrap();
        let mut ps = vec![];
        paragraphs(&doc.blocks[0], &mut ps);
        for (text, weight) in [
            ("Heading", gpui::FontWeight::BOLD),
            ("Plain heading", gpui::FontWeight::NORMAL),
        ] {
            let node = ps
                .iter()
                .flat_map(|p| &p.children)
                .find(|n| n.text.as_ref() == text)
                .unwrap();
            assert_eq!(node.marks[0].1.font_weight, Some(weight));
        }
        let code = ps
            .iter()
            .flat_map(|p| &p.children)
            .find(|n| n.text.as_ref() == "code")
            .unwrap();
        let mark = &code.marks[0].1;
        assert!(mark.code);
        assert!(mark.font_family.is_none());
        assert_eq!(mark.font_size_scale, Some(1.));
        let styled = ps
            .iter()
            .flat_map(|p| &p.children)
            .find(|n| n.text.as_ref() == "styled")
            .unwrap();
        assert_eq!(
            styled.marks[0].1.font_family.as_deref(),
            Some("Example Sans")
        );
    }

    #[test]
    fn test_cleanup_html() {
        let html = r#"<p>
            and
            <code>code</code>
            text
        </p>"#;
        let cleaned = super::cleanup_html(html);
        assert_eq!(
            String::from_utf8(cleaned).unwrap(),
            "<p>and <code>code</code> text"
        );

        let html = r#"<p>
            and
            <em>   <code>code</code>   <i>italic</i>   </em>
            text
        </p>"#;
        let cleaned = super::cleanup_html(html);
        assert_eq!(
            String::from_utf8(cleaned).unwrap(),
            "<p>and <em><code>code</code> <i>italic</i></em> text"
        );
    }

    #[test]
    fn test_trim_text() {
        assert_eq!(trim_text("  \n\tHello world \t\r "), " Hello world ",);
    }

    #[test]
    fn test_mark() {
        let mut cx = NodeContext::default();

        // `<mark>` is rendered as a highlight, kept as `==...==` in markdown.
        let html = r#"<p>Hello <mark>world</mark></p>"#;
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(node.to_markdown(), "Hello ==world==");

        let html = r#"<p><mark color="blue">blue</mark> and <mark style="background-color: #336699">hex</mark></p>"#;
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(node.to_markdown(), "==blue== and ==hex==");
    }

    #[test]
    fn test_keep_spaces() {
        let html = r#"<p>and <code>code</code> text</p>"#;
        let mut cx = NodeContext::default();
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(node.to_markdown(), "and `code` text");

        let html = r#"
            <div>
            <p>
                and
                <em>   <code>code</code>   <i>italic</i>   </em>
                text
            </p>
            <p>
                <img src="https://example.com/image.png" alt="Example" width="100" height="200" title="Example Image" />
            </p>
            <ul>
                <li>Item 1</li>
                <li>Item 2
                </li>
            </ul>
            </div>
        "#;
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(
            node.to_markdown(),
            indoc::indoc! {r#"
            and *`code` italic* text

            ![Example](https://example.com/image.png "Example Image")

            - Item 1
            - Item 2
            "#}
            .trim()
        );
    }

    #[test]
    fn test_value_to_length() {
        assert_eq!(super::value_to_length("100px"), Some(px(100.).into()));
        assert_eq!(super::value_to_length("100%"), Some(relative(1.)));
        assert_eq!(super::value_to_length("56%"), Some(relative(0.56)));
        assert_eq!(super::value_to_length("240"), Some(px(240.).into()));
    }

    #[test]
    fn test_image() {
        let html = r#"<img src="https://example.com/image.png" alt="Example" width="100" height="200" title="Example Image" />"#;
        let mut cx = NodeContext::default();
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(
            node,
            ParsedDocument {
                source: html.to_string().into(),
                blocks: Arc::new(vec![BlockNode::Paragraph(Paragraph {
                    span: None,
                    children: vec![InlineNode::image(ImageNode {
                        url: "https://example.com/image.png".to_string().into(),
                        alt: Some("Example".to_string().into()),
                        width: Some(px(100.).into()),
                        height: Some(px(200.).into()),
                        title: Some("Example Image".to_string().into()),
                        ..Default::default()
                    })],
                    ..Default::default()
                })])
            }
        );

        let html = r#"<img src="https://example.com/image.png" alt="Example" style="width: 80%" title="Example Image" />"#;
        let node = super::parse(html, &mut cx).unwrap();
        assert_eq!(
            node,
            ParsedDocument {
                source: html.to_string().into(),
                blocks: Arc::new(vec![BlockNode::Paragraph(Paragraph {
                    span: None,
                    children: vec![InlineNode::image(ImageNode {
                        url: "https://example.com/image.png".to_string().into(),
                        alt: Some("Example".to_string().into()),
                        width: Some(relative(0.8)),
                        height: None,
                        title: Some("Example Image".to_string().into()),
                        ..Default::default()
                    })],
                    ..Default::default()
                })])
            }
        );
    }
}
