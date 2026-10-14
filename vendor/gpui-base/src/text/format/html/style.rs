//! Small, bounded HTML presentation model. This is not a browser CSS cascade.
use super::*;
use gpui::{FontStyle, FontWeight};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HtmlParagraphStyle {
    pub font_size: f32,
    pub line_height: f32,
    pub align: String,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub preserve: bool,
}

/// Relative units use the inherited font; rem is the native HTML base (16px).
/// Percent spacing is deliberately font-relative, not viewport-relative.
fn length(value: &str, inherited: f32) -> Option<f32> {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    let (number, factor) = if let Some(v) = value.strip_suffix("rem") {
        (v, 16.)
    } else if let Some(v) = value.strip_suffix("em") {
        (v, inherited)
    } else if let Some(v) = value.strip_suffix("pt") {
        (v, 4. / 3.)
    } else if let Some(v) = value.strip_suffix("px") {
        (v, 1.)
    } else if let Some(v) = value.strip_suffix('%') {
        (v, inherited / 100.)
    } else if value == "0" {
        ("0", 1.)
    } else {
        return None;
    };
    let result = number.trim().parse::<f32>().ok()? * factor;
    result.is_finite().then_some(result)
}

pub(super) struct Resolved {
    pub paragraph: HtmlParagraphStyle,
    pub mark: TextMark,
    pub preserve: bool,
}

pub(super) fn resolve(node: &Rc<Node>) -> Resolved {
    // rcdom's parent link is a Cell; restore it immediately after inspection.
    let mut ancestors = vec![node.clone()];
    let mut current = node.clone();
    for _ in 0..128 {
        let parent = current.parent.take();
        let next = parent.as_ref().and_then(|p| p.upgrade());
        current.parent.set(parent);
        let Some(next) = next else { break };
        ancestors.push(next.clone());
        current = next;
    }
    ancestors.reverse();
    let mut font_size: f32 = 16.;
    let mut block_font = 16.;
    let mut line_height = 1.4;
    let mut absolute_line_height = None;
    let mut align = String::new();
    let mut top = 0.;
    let mut bottom = 0.;
    let mut left = 0.;
    let mut right = 0.;
    let mut preserve = false;
    let mut mark = TextMark::default();
    let mut html_code = false;
    for node in ancestors {
        let NodeData::Element { name, attrs, .. } = &node.data else {
            continue;
        };
        let tag: &str = &name.local;
        let css = attr_value(attrs, local_name!("style"))
            .map(|v| style_attrs(&v))
            .unwrap_or_default();
        let inherited_font = font_size;
        font_size *= match tag {
            "h1" => 2.,
            "h2" => 1.5,
            "h3" => 1.25,
            "h4" => 1.125,
            "h6" => 0.875,
            "small" => 0.8333333,
            _ => 1.,
        };
        font_size = font_size.clamp(8., 64.);
        if let Some(size) = css.get("font-size").and_then(|v| length(v, inherited_font)) {
            font_size = size.clamp(8., 64.);
        } else if tag == "font" {
            if let Some(size) = attr_value(attrs, local_name!("size")).and_then(|v| {
                let size = v.trim().parse::<i32>().ok()?;
                Some(if v.trim().starts_with(['+', '-']) {
                    3 + size
                } else {
                    size
                })
            }) {
                font_size = [8., 10., 13., 16., 18., 24., 32., 48.][size.clamp(1, 7) as usize];
            }
        }
        if let Some(color) = css
            .get("color")
            .and_then(|v| parse_css_color(v))
            .or_else(|| attr_value(attrs, local_name!("color")).and_then(|v| parse_css_color(&v)))
        {
            mark.color = Some(color);
        }
        if tag == "code" {
            // The semantic monospace default is specified on code itself, not
            // inherited from the surrounding sender font. Explicit code CSS wins.
            mark.font_family = None;
            html_code = true;
        }
        if let Some(family) = css
            .get("font-family")
            .cloned()
            .or_else(|| attr_value(attrs, local_name!("face")))
        {
            if let Some(first) = family.split(',').next() {
                let family = first.trim().trim_matches(['\'', '"']);
                if !family.is_empty() && family.len() <= 128 {
                    mark.font_family = Some(family.to_owned().into());
                }
            }
        }
        if matches!(
            tag,
            "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            mark.font_weight = Some(FontWeight::BOLD);
        }
        if tag == "i" || tag == "em" {
            mark.font_style = Some(FontStyle::Italic);
        }
        if let Some(weight) = css.get("font-weight") {
            mark.font_weight = match weight.trim() {
                "normal" => Some(FontWeight::NORMAL),
                "bold" => Some(FontWeight::BOLD),
                v => v
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(|v| FontWeight(v.clamp(100., 900.))),
            }
            .or(mark.font_weight);
        }
        if let Some(style) = css.get("font-style") {
            mark.font_style = match style.trim() {
                "normal" => Some(FontStyle::Normal),
                "italic" | "oblique" => Some(FontStyle::Italic),
                _ => mark.font_style,
            };
        }
        if let Some(decoration) = css.get("text-decoration") {
            mark.underline |= decoration.contains("underline");
            mark.strikethrough |= decoration.contains("line-through");
        }
        if let Some(value) = css.get("white-space") {
            match value.trim() {
                "pre" | "pre-wrap" => preserve = true,
                "normal" => preserve = false,
                _ => {}
            }
        }
        if let Some(value) = css
            .get("text-align")
            .cloned()
            .or_else(|| attr_value(attrs, local_name!("align")))
        {
            align = value.trim().to_owned();
        }
        if let Some(value) = css.get("line-height") {
            let number = value.parse::<f32>().ok().filter(|n| n.is_finite());
            if value.trim() == "normal" {
                line_height = 1.4;
                absolute_line_height = None;
            } else if let Some(value) = number {
                line_height = value.clamp(0.8, 4.);
                absolute_line_height = None;
            } else if let Some(value) = length(value, font_size) {
                absolute_line_height = Some(value.clamp(8., 256.));
            }
        }
        if BLOCK_ELEMENTS.contains(&tag) || tag == "li" || tag == "td" || tag == "th" {
            block_font = font_size;
            top = if tag == "p" { font_size * 0.5 } else { 0. };
            bottom = top;
            left = 0.;
            right = 0.;
            // Process declarations in authored order so shorthand and longhand cascade.
            if let Some(style) = attr_value(attrs, local_name!("style")) {
                for decl in style.split(';') {
                    let Some((key, value)) = decl.split_once(':') else {
                        continue;
                    };
                    match key.trim().to_ascii_lowercase().as_str() {
                        "margin" => {
                            let value = value.trim().trim_end_matches("!important").trim();
                            let values: Option<Vec<_>> = value
                                .split_whitespace()
                                .map(|v| length(v, font_size))
                                .collect();
                            if let Some(values) = values.filter(|v| (1..=4).contains(&v.len())) {
                                top = values[0].clamp(0., 96.);
                                bottom = values.get(2).copied().unwrap_or(values[0]).clamp(0., 96.);
                                right = values.get(1).copied().unwrap_or(values[0]).clamp(0., 96.);
                                left = values.get(3).copied().unwrap_or(right).clamp(0., 96.);
                            }
                        }
                        "margin-top" => {
                            if let Some(v) = length(value, font_size) {
                                top = v.clamp(0., 96.);
                            }
                        }
                        "margin-bottom" => {
                            if let Some(v) = length(value, font_size) {
                                bottom = v.clamp(0., 96.);
                            }
                        }
                        "margin-left" => {
                            if let Some(v) = length(value, font_size) {
                                left = v.clamp(0., 96.);
                            }
                        }
                        "margin-right" => {
                            if let Some(v) = length(value, font_size) {
                                right = v.clamp(0., 96.);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if let Some(height) = absolute_line_height {
        line_height = height / block_font;
    }
    mark.font_size_scale = (font_size != block_font || html_code).then_some(font_size / block_font);
    Resolved {
        paragraph: HtmlParagraphStyle {
            font_size: block_font,
            line_height,
            align,
            margin_top: top,
            margin_bottom: bottom,
            margin_left: left,
            margin_right: right,
            preserve,
        },
        mark,
        preserve,
    }
}

pub(super) fn push_text(paragraph: &mut Paragraph, node: &Rc<Node>, text: &str) {
    let style = resolve(node);
    let block = paragraph.html_style.get_or_insert(style.paragraph);
    block.preserve |= style.preserve;
    let text = if style.preserve {
        text.to_owned()
    } else {
        let mut out = String::new();
        let mut space = paragraph
            .children
            .last()
            .is_some_and(|n| n.text.ends_with([' ', '\n']));
        for c in text.chars() {
            if matches!(c, ' ' | '\n' | '\r' | '\t' | '\u{c}') {
                if !space {
                    out.push(' ');
                }
                space = true;
            } else {
                out.push(c);
                space = false;
            }
        }
        out
    };
    if !text.is_empty() {
        paragraph.push(InlineNode::new(text.clone()).marks(vec![(0..text.len(), style.mark)]));
    }
}
