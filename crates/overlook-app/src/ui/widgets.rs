//! Small presentational helpers shared by the mail views.

use crate::theme::Palette;
use gpui_kit::*;
use overlook_core::Address;

/// Circular initials avatar in the brand's muted green-gray.
pub fn avatar(address: &Address, size: f32, p: &Palette) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(p.avatar)
        .text_color(p.avatar_ink)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size * 0.36).round()))
        .font_weight(FontWeight::SEMIBOLD)
        .child(address.initials())
}

/// Avatar for the account owner, in the selection tint like the preview's "JA" badge.
pub fn owner_avatar(address: &Address, size: f32, p: &Palette) -> Div {
    avatar(address, size, p).bg(p.selection).text_color(p.selection_text)
}

/// Quiet rounded chip for labels, counts, and keyboard hints.
pub fn chip(text: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex_none()
        .px(px(7.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(p.chip)
        .border_1()
        .border_color(p.divider)
        .text_size(px(11.))
        .text_color(p.muted)
        .child(text.into())
}

/// Keyboard hint, e.g. `Ctrl E`.
pub fn kbd(text: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(4.))
        .border_1()
        .border_color(p.divider)
        .bg(p.surface)
        .text_size(px(11.))
        .text_color(p.muted)
        .child(text.into())
}

/// Small colored dot for user labels.
pub fn color_dot(color: Hsla) -> Div {
    div().flex_none().size(px(7.)).rounded_full().bg(color)
}

/// Thin horizontal divider.
pub fn divider(p: &Palette) -> Div {
    div().w_full().h(px(1.)).bg(p.divider)
}
