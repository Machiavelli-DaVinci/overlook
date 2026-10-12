//! Account and folder navigation.

use super::widgets::*;
use super::{MailWindow, Nav};
use crate::assets::icon;
use crate::theme::ActivePalette;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::{h_flex, v_flex};
use overlook_core::*;
use overlook_store::MailboxView;

impl MailWindow {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let status = self.status.clone();
        let outbox = status.outbox_items;
        let drafts = self.drafts.len();

        let mut favorites: Vec<Nav> = vec![
            Nav::view("Inbox", "inbox", MailboxView::Kind(LabelKind::Inbox)),
            Nav::view("Starred", "star", MailboxView::Kind(LabelKind::Starred)),
            Nav::view("Drafts", "square-pen", MailboxView::Kind(LabelKind::Drafts)),
        ];
        if outbox > 0 || matches!(self.view, MailboxView::Kind(LabelKind::Outbox)) {
            favorites.push(Nav::view("Outbox", "clock", MailboxView::Kind(LabelKind::Outbox)));
        }
        favorites.extend([
            Nav::view("Sent", "send", MailboxView::Kind(LabelKind::Sent)),
            Nav::view("Archive", "archive", MailboxView::Kind(LabelKind::Archive)),
            Nav::view("Trash", "trash", MailboxView::Kind(LabelKind::Trash)),
        ]);

        let nav_button = |this: &Self, nav: Nav, cx: &mut Context<Self>| {
            let current = this.view == nav.view && this.search_query.is_empty();
            let count = match &nav.view {
                MailboxView::Kind(LabelKind::Drafts) => drafts,
                MailboxView::Kind(LabelKind::Outbox) => outbox,
                v => this.counts.get(v).copied().unwrap_or(0),
            };
            let view = nav.view.clone();
            let color = nav.color;
            div()
                .id(SharedString::from(format!("nav-{}", nav.label)))
                .h_flex()
                .items_center()
                .gap(px(10.))
                .min_h(px(35.))
                .px(px(11.))
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if current { p.accent } else { p.foreground })
                .font_weight(if current { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .when(current, |d| d.bg(p.selection))
                .when(!current, |d| d.hover(|d| d.bg(p.hover)))
                .child(match color {
                    Some(c) => color_dot(c).mx(px(4.)).into_any_element(),
                    None => icon(nav.icon)
                        .size(px(16.))
                        .text_color(if current { p.accent } else { p.muted })
                        .into_any_element(),
                })
                .child(div().flex_1().truncate().child(nav.label.clone()))
                .when(count > 0, |d| {
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(if current { p.accent } else { p.muted })
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| this.select_view(view.clone(), window, cx)))
        };

        let mut labels_section = v_flex().gap(px(3.));
        for label in self.labels.iter().filter(|l| l.kind == LabelKind::User) {
            let nav = Nav {
                label: label.name.clone().into(),
                icon: "tag",
                view: MailboxView::Label(label.id.clone()),
                color: Some(p.label_color(label.color.as_deref())),
            };
            labels_section = labels_section.child(nav_button(self, nav, cx));
        }

        let owner = self.owner();
        let state_color = match status.state {
            SyncState::UpToDate => p.success,
            SyncState::Syncing => p.accent,
            SyncState::Offline => p.muted,
            SyncState::SignInRequired | SyncState::SyncFailed(_) => p.warning,
        };

        v_flex()
            .size_full()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.divider)
            .pt(px(18.))
            .px(px(11.))
            .pb(px(14.))
            .child(
                Button::new("new-message")
                    .primary()
                    .icon(icon("square-pen").size(px(16.)))
                    .label("New message")
                    .tooltip("New message (Ctrl N)")
                    .w_full()
                    .h(px(38.))
                    .on_click(cx.listener(|this, _, window, cx| this.compose_new(window, cx))),
            )
            .child(section_label("Favorites", &p))
            .child(v_flex().gap(px(3.)).children(favorites.into_iter().map(|nav| nav_button(self, nav, cx))))
            .child(section_label("Labels", &p))
            .child(labels_section)
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap(px(9.))
                    .px(px(6.))
                    .pt(px(16.))
                    .child(owner_avatar(&owner, 30., &p))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).truncate().child(owner.display_name()))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .text_size(px(11.))
                                    .text_color(p.muted)
                                    .child(div().size(px(6.)).rounded_full().bg(state_color))
                                    .child(format!("{} · {}", self.account.provider.label(), status.state.label())),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("sync-now")
                            .ghost()
                            .xsmall()
                            .icon(icon("refresh-cw").size(px(14.)))
                            .tooltip("Sync now")
                            .loading(self.syncing)
                            .on_click(cx.listener(|this, _, window, cx| this.sync_now(window, cx))),
                    ),
            )
    }
}
