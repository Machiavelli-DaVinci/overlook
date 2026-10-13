//! Reading pane: conversation header, collapsed history, expanded message, attachments.

use super::widgets::*;
use super::MailWindow;
use crate::assets::icon;
use crate::theme::ActivePalette;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::{h_flex, v_flex};
use overlook_core::sanitize::{sanitize_html, text_to_html, RemoteContent};
use overlook_core::time::{format_full, relative_label};
use overlook_core::*;
use overlook_store::MailboxView;

impl MailWindow {
    pub(super) fn render_reader(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let container = v_flex().size_full().bg(p.surface).min_w_0();

        if let Some(composer) = &self.composer {
            if composer.draft.kind == DraftKind::New || self.selected.is_none() {
                return container.child(self.render_composer(cx)).into_any_element();
            }
        }

        if let Some(item) = self.outbox_selected.as_ref().and_then(|id| self.outbox.iter().find(|d| &d.id == id)) {
            return container.child(self.render_outbox_item(item, cx)).into_any_element();
        }

        let Some(summary) = self.selected_summary() else {
            return container
                .child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .child(svg().path("overlook/symbolic.svg").size(px(44.)).text_color(p.divider))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child("Select a conversation to read it here."))
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .items_center()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(kbd("J", &p))
                                .child(kbd("K", &p))
                                .child("move")
                                .child(kbd("E", &p))
                                .child("archive")
                                .child(kbd("Ctrl N", &p))
                                .child("new message"),
                        ),
                )
                .into_any_element();
        };

        let selected_ids: Vec<MessageId> = self.thread.iter().map(|m| m.id.clone()).collect();
        let unread_any = self.thread.iter().any(|m| !m.is_read);
        let (in_trash, in_inbox) = self.thread_state();
        let toolbar = h_flex()
            .items_center()
            .gap(px(2.))
            .h(px(48.))
            .px(px(12.))
            .overflow_hidden()
            .border_b_1()
            .border_color(p.divider)
            .when(in_trash, |bar| {
                bar.child(toolbar_button("inbox", "Restore", "Restore to Inbox (E)", cx.listener({
                    let ids = selected_ids.clone();
                    move |this, _, window, cx| this.restore(ids.clone(), window, cx)
                })))
            })
            .when(!in_trash && !in_inbox, |bar| {
                bar.child(toolbar_button("inbox", "Move to Inbox", "Move to Inbox (E)", cx.listener({
                    let ids = selected_ids.clone();
                    move |this, _, window, cx| this.move_to_inbox(ids.clone(), window, cx)
                })))
            })
            .when(!in_trash && in_inbox, |bar| {
                bar.child(toolbar_button("archive", "Archive", "Archive (E)", cx.listener({
                    let ids = selected_ids.clone();
                    move |this, _, window, cx| this.archive(ids.clone(), window, cx)
                })))
            })
            .when(!in_trash, |bar| {
                bar.child(toolbar_button("trash", "Delete", "Delete (Del)", cx.listener({
                    let ids = selected_ids.clone();
                    move |this, _, window, cx| this.trash(ids.clone(), window, cx)
                })))
            })
            .child({
                let labels: Vec<(LabelId, String)> = self.labels.iter().filter(|l| l.kind == LabelKind::User).map(|l| (l.id.clone(), l.name.clone())).collect();
                let ids = selected_ids.clone();
                let t = cx.entity().downgrade();
                Button::new("move-to")
                    .ghost()
                    .small()
                    .icon(icon("folder-input").size(px(16.)))
                    .label("Move to")
                    .dropdown_menu(move |mut menu, _window, _cx| {
                        for (label, view) in [("Inbox", MailboxView::Kind(LabelKind::Inbox)), ("Archive", MailboxView::Kind(LabelKind::Archive)), ("Trash", MailboxView::Kind(LabelKind::Trash))] {
                            let t = t.clone();
                            let ids = ids.clone();
                            menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                let drag = super::ConversationDrag { message_ids: ids.clone(), title: String::new(), count: 0 };
                                let _ = t.update(cx, |this, cx| this.drop_on_view(drag, view.clone(), window, cx));
                            }));
                        }
                        if !labels.is_empty() {
                            menu = menu.separator().label("Labels");
                        }
                        for (id, name) in &labels {
                            let t = t.clone();
                            let ids = ids.clone();
                            let id = id.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| {
                                let _ = t.update(cx, |this, cx| this.move_to_label(ids.clone(), id.clone(), window, cx));
                            }));
                        }
                        menu
                    })
            })
            .child(toolbar_button(
                if unread_any { "mail-open" } else { "mail" },
                if unread_any { "Mark read" } else { "Mark unread" },
                "Toggle read (Ctrl U)",
                cx.listener({
                    let ids = selected_ids.clone();
                    move |this, _, window, cx| this.set_read(ids.clone(), unread_any, window, cx)
                }),
            ))
            .when(self.focused_enabled && self.view == overlook_store::MailboxView::Kind(LabelKind::Inbox), |bar| {
                let in_focused = self.inbox_tab == FocusTab::Focused;
                let target = if in_focused { "Other" } else { "Focused" };
                let sender = self.thread.iter().filter(|m| !m.is_outgoing).max_by_key(|m| m.date).map(|m| m.from.display_name());
                let t = cx.entity().downgrade();
                bar.child(
                    Button::new("move-focus")
                        .ghost()
                        .small()
                        .icon(icon("arrow-right-left").size(px(16.)))
                        .label(format!("Move to {target}"))
                        .dropdown_menu(move |menu, _window, _cx| {
                            let a = t.clone();
                            let b = t.clone();
                            let move_to = !in_focused;
                            let mut menu = menu.item(PopupMenuItem::new(format!("Move this conversation to {target}")).on_click(move |_, window, cx| {
                                let _ = a.update(cx, |this, cx| this.move_focus(move_to, false, window, cx));
                            }));
                            if let Some(name) = &sender {
                                menu = menu.item(PopupMenuItem::new(format!("Always move messages from {name} to {target}")).on_click(move |_, window, cx| {
                                    let _ = b.update(cx, |this, cx| this.move_focus(move_to, true, window, cx));
                                }));
                            }
                            menu
                        }),
                )
            })
            .child(div().flex_1())
            .child(toolbar_button("reply", "Reply", "Reply (Ctrl R)", cx.listener(|this, _, window, cx| this.reply(DraftKind::Reply, window, cx))))
            .child(toolbar_button("reply-all", "Reply all", "Reply all (Ctrl Shift R)", cx.listener(|this, _, window, cx| this.reply(DraftKind::ReplyAll, window, cx))))
            .child(toolbar_button("forward", "Forward", "Forward (Ctrl F)", cx.listener(|this, _, window, cx| this.reply(DraftKind::Forward, window, cx))))
            .child(div().w(px(2.)))
            .child(
                Button::new("prev-conv")
                    .ghost()
                    .small()
                    .icon(icon("chevron-left").size(px(16.)))
                    .tooltip("Previous (K)")
                    .on_click(cx.listener(|this, _, window, cx| this.move_selection(-1, window, cx))),
            )
            .child(
                Button::new("next-conv")
                    .ghost()
                    .small()
                    .icon(icon("chevron-right").size(px(16.)))
                    .tooltip("Next (J)")
                    .on_click(cx.listener(|this, _, window, cx| this.move_selection(1, window, cx))),
            );

        let labels: Vec<String> = summary
            .label_ids
            .iter()
            .filter_map(|id| self.labels.iter().find(|l| &l.id == id))
            .filter(|l| l.kind == LabelKind::User)
            .map(|l| l.name.clone())
            .collect();

        let header = v_flex()
            .px(px(28.))
            .pt(px(22.))
            .pb(px(14.))
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(24.))
                    .line_height(px(32.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(summary.subject.clone()),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .children(labels.into_iter().map(|l| chip(l, &p).text_color(p.accent)))
                    .child(format!(
                        "{} message{}",
                        summary.message_count,
                        if summary.message_count == 1 { "" } else { "s" }
                    ))
                    .child("·")
                    .child(format!("You and {}", summary.participant_line()))
                    .when(self.thread.len() > 1, |d| {
                        let all_ids: Vec<MessageId> = self.thread.iter().map(|m| m.id.clone()).collect();
                        let all_open = all_ids.iter().all(|id| self.expanded.contains(id) || *id == summary.latest_message_id);
                        d.child("·").child(
                            Button::new("expand-all")
                                .ghost()
                                .xsmall()
                                .icon(icon(if all_open { "chevrons-down-up" } else { "chevrons-up-down" }).size(px(13.)))
                                .label(if all_open { "Collapse all" } else { "Expand all" })
                                .on_click(cx.listener(move |this, _, _w, cx| {
                                    if all_open {
                                        this.expanded.clear();
                                    } else {
                                        this.expanded.extend(all_ids.iter().cloned());
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(summary.pending, |d| {
                        d.child("·").child(
                            h_flex()
                                .gap(px(4.))
                                .items_center()
                                .text_color(p.warning)
                                .child(icon("cloud-off").size(px(12.)).text_color(p.warning))
                                .child("Changes saved on this device, waiting to sync"),
                        )
                    }),
            )
            .child(divider(&p));

        let latest_id = summary.latest_message_id.clone();
        let mut messages = v_flex().px(px(28.)).gap(px(6.));
        for m in &self.thread {
            let expanded = m.id == latest_id || self.expanded.contains(&m.id);
            messages = messages.child(if expanded {
                self.render_message_expanded(m, cx).into_any_element()
            } else {
                self.render_message_collapsed(m, cx).into_any_element()
            });
        }

        let reply_area: AnyElement = match &self.composer {
            Some(c) if c.draft.conversation_id.as_ref() == Some(&summary.id) => self.render_composer(cx).into_any_element(),
            _ => {
                let who = summary.participants.first().map(Address::display_name).unwrap_or_else(|| "sender".into());
                v_flex()
                    .mx(px(28.))
                    .mt(px(18.))
                    .mb(px(28.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.divider)
                    .child(
                        div()
                            .id("reply-box")
                            .h_flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .h(px(46.))
                            .text_size(px(13.))
                            .text_color(p.muted)
                            .cursor_text()
                            .child(icon("corner-up-left").size(px(16.)).text_color(p.muted))
                            .child(format!("Reply to {who}…"))
                            .on_click(cx.listener(|this, _, window, cx| this.reply_default(window, cx))),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(4.))
                            .px(px(8.))
                            .py(px(6.))
                            .border_t_1()
                            .border_color(p.divider)
                            .child(
                                Button::new("reply-btn")
                                    .ghost()
                                    .small()
                                    .icon(icon("reply").size(px(15.)))
                                    .label("Reply")
                                    .on_click(cx.listener(|this, _, window, cx| this.reply(DraftKind::Reply, window, cx))),
                            )
                            .child(
                                Button::new("forward-btn")
                                    .ghost()
                                    .small()
                                    .icon(icon("forward").size(px(15.)))
                                    .label("Forward")
                                    .on_click(cx.listener(|this, _, window, cx| this.reply(DraftKind::Forward, window, cx))),
                            )
                            .child(div().flex_1())
                            .child(kbd("Ctrl R", &p)),
                    )
                    .into_any_element()
            }
        };

        container
            .child(toolbar)
            .child(
                v_flex()
                    .id("reader-scroll")
                    .flex_1()
                    .min_h_0()
                    .child(header)
                    .child(messages)
                    .child(reply_area)
                    .overflow_y_scrollbar(),
            )
            .into_any_element()
    }

    fn render_message_collapsed(&self, m: &Message, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let id = m.id.clone();
        let now = overlook_core::time::now();
        let who = if m.is_outgoing { Address::named("You", &self.account.email) } else { m.from.clone() };
        let owner = self.owner();
        div()
            .id(SharedString::from(format!("collapsed-{}", m.id)))
            .h_flex()
            .items_center()
            .gap(px(12.))
            .px(px(6.))
            .py(px(10.))
            .rounded(px(8.))
            .border_b_1()
            .border_color(p.divider)
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(if m.is_outgoing { owner_avatar(&owner, 28., &p) } else { avatar(&who, 28., &p) })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.)).font_weight(if m.is_read { FontWeight::MEDIUM } else { FontWeight::SEMIBOLD }).child(who.display_name()))
                    .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(m.snippet.clone())),
            )
            .child(div().text_size(px(11.)).text_color(p.muted).child(relative_label(m.date, now, self.utc_offset)))
            .on_click(cx.listener(move |this, _, _window, cx| {
                if !this.expanded.insert(id.clone()) {
                    this.expanded.remove(&id);
                }
                cx.notify();
            }))
    }

    fn render_message_expanded(&self, m: &Message, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let who = if m.is_outgoing { Address::named("You", &self.account.email) } else { m.from.clone() };
        let owner = self.owner();
        let allow_remote = self.remote_allowed.contains(&m.id) || self.prefs.remote_images == RemoteImages::Allow;
        let policy = if allow_remote { RemoteContent::Allowed } else { RemoteContent::Blocked };
        let html = match (&m.body.html, &m.body.text) {
            (Some(h), _) => h.clone(),
            (None, Some(t)) => text_to_html(t),
            (None, None) => String::new(),
        };
        let sanitized = sanitize_html(&html, policy);
        let dark = p.is_dark();
        // Quoted history is folded away only when it repeats a message already in this
        // conversation; otherwise it is part of what the sender wants read.
        let (body, quoted) = match sanitized.quoted {
            Some(q) if self.quote_repeats_thread(&q, m) => (sanitized.html, Some(q)),
            Some(q) => (format!("{}{}", sanitized.html, q), None),
            None => (sanitized.html, None),
        };
        let body_html = self.resolve_inline_images(m, overlook_core::sanitize::adapt_colors(&body, dark));
        let quoted_html = quoted.map(|q| self.resolve_inline_images(m, overlook_core::sanitize::adapt_colors(&q, dark)));
        let quote_shown = self.quotes_shown.contains(&m.id);
        let quote_id = m.id.clone();
        let to_line = {
            let mut parts: Vec<String> = m
                .to
                .iter()
                .map(|a| if a.email.eq_ignore_ascii_case(&self.account.email) { "me".to_owned() } else { a.display_name() })
                .collect();
            if parts.is_empty() {
                parts.push("me".into());
            }
            format!("to {} · {}", parts.join(", "), m.from.email)
        };
        let mid = m.id.clone();
        let is_latest = self.selected_summary().is_some_and(|s| s.latest_message_id == m.id);
        let collapse_id = m.id.clone();
        let menu_message = m.clone();
        let t = cx.entity().downgrade();

        v_flex()
            .pt(px(10.))
            .pb(px(6.))
            .gap(px(14.))
            .child(
                h_flex()
                    .id(SharedString::from(format!("mhead-{}", m.id)))
                    .items_center()
                    .gap(px(12.))
                    .when(!is_latest, |d| d.cursor_pointer())
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        if !is_latest {
                            this.expanded.remove(&collapse_id);
                            cx.notify();
                        }
                    }))
                    .child(if m.is_outgoing { owner_avatar(&owner, 34., &p) } else { avatar(&who, 34., &p) })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child({
                                let card_id = m.id.clone();
                                div()
                                    .id(SharedString::from(format!("sender-{}", m.id)))
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .hover(|d| d.text_color(p.accent))
                                    .child(who.display_name())
                                    .on_click(cx.listener(move |this, _, _w, cx| {
                                        cx.stop_propagation();
                                        this.contact_card = if this.contact_card.as_ref() == Some(&card_id) { None } else { Some(card_id.clone()) };
                                        cx.notify();
                                    }))
                            })
                            .child(div().truncate().text_size(px(12.)).text_color(p.muted).child(to_line)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(format_full(m.date, self.utc_offset)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("mmenu-{}", m.id)))
                            .ghost()
                            .xsmall()
                            .icon(icon("ellipsis-vertical").size(px(15.)))
                            .tooltip("More actions for this message")
                            .dropdown_menu(move |menu, _window, _cx| {
                                let m = menu_message.clone();
                                let item = |menu: gpui_kit::component::menu::PopupMenu, label: &str, t: WeakEntity<MailWindow>, m: Message, f: fn(&mut MailWindow, Message, &mut Window, &mut Context<MailWindow>)| {
                                    menu.item(PopupMenuItem::new(label.to_owned()).on_click(move |_, window, cx| {
                                        let _ = t.update(cx, |this, cx| f(this, m.clone(), window, cx));
                                    }))
                                };
                                let menu = item(menu, "Reply", t.clone(), m.clone(), |this, m, w, cx| this.reply_to(DraftKind::Reply, m, w, cx));
                                let menu = item(menu, "Reply all", t.clone(), m.clone(), |this, m, w, cx| this.reply_to(DraftKind::ReplyAll, m, w, cx));
                                let menu = item(menu, "Forward", t.clone(), m.clone(), |this, m, w, cx| this.reply_to(DraftKind::Forward, m, w, cx));
                                let menu = menu.separator();
                                let menu = item(menu, if m.is_read { "Mark as unread" } else { "Mark as read" }, t.clone(), m.clone(), |this, m, w, cx| {
                                    let read = !m.is_read;
                                    this.set_read(vec![m.id.clone()], read, w, cx)
                                });
                                let menu = menu.separator();
                                let menu = item(menu, "Print…", t.clone(), m.clone(), |this, m, w, cx| this.print_message(m, w, cx));
                                let menu = item(menu, "Save as .eml…", t.clone(), m.clone(), |this, m, w, cx| this.save_message_eml(m, w, cx));
                                item(menu, "View source", t.clone(), m, |this, m, w, cx| this.view_message_source(m, w, cx))
                            }),
                    ),
            )
            .when(self.contact_card.as_ref() == Some(&m.id), |d| d.child(self.render_contact_card(&who, cx)))
            .when(!m.body.is_cached(), |d| {
                d.child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(p.chip)
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child("This message hasn’t been downloaded. Connect to read it."),
                )
            })
            .children(m.invitation.as_ref().map(|inv| self.render_invitation_card(inv, cx)))
            .when(sanitized.blocked_remote > 0, |d| {
                d.child(
                    h_flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(p.chip)
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(icon("image-off").size(px(14.)).text_color(p.muted))
                        .child(format!(
                            "{} remote image{} blocked to protect your privacy.",
                            sanitized.blocked_remote,
                            if sanitized.blocked_remote == 1 { "" } else { "s" }
                        ))
                        .child(div().flex_1())
                        .child(
                            Button::new(SharedString::from(format!("allow-remote-{}", m.id)))
                                .ghost()
                                .xsmall()
                                .label("Show images")
                                .on_click(cx.listener(move |this, _, _window, cx| {
                                    this.remote_allowed.insert(mid.clone());
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .max_w(px(760.))
                    .child(
                        TextView::html(SharedString::from(format!("body-{}-{}", m.id, allow_remote)), body_html)
                            .selectable(true),
                    ),
            )
            .when_some(quoted_html, |d, quoted| {
                d.child(
                    v_flex()
                        .gap(px(8.))
                        .child(
                            div().child(
                                Button::new(SharedString::from(format!("quote-{}", m.id)))
                                    .ghost()
                                    .xsmall()
                                    .icon(icon("ellipsis").size(px(14.)))
                                    .label(if quote_shown { "Hide quoted text" } else { "Show quoted text" })
                                    .tooltip("The earlier messages this one replies to")
                                    .on_click(cx.listener(move |this, _, _w, cx| {
                                        if !this.quotes_shown.remove(&quote_id) {
                                            this.quotes_shown.insert(quote_id.clone());
                                        }
                                        cx.notify();
                                    })),
                            ),
                        )
                        .when(quote_shown, |d| {
                            d.child(
                                div()
                                    .pl(px(14.))
                                    .border_l_2()
                                    .border_color(p.divider)
                                    .text_size(px(15.))
                                    .line_height(px(24.))
                                    .max_w(px(760.))
                                    .text_color(p.muted)
                                    .child(TextView::html(SharedString::from(format!("quoted-{}-{}", m.id, allow_remote)), quoted).selectable(true)),
                            )
                        }),
                )
            })
            .when(!m.attachments.is_empty(), |d| {
                d.child(
                    v_flex()
                        .gap(px(10.))
                        .pt(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("{} attachment{}", m.attachments.len(), if m.attachments.len() == 1 { "" } else { "s" })),
                        )
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap(px(10.))
                                .children(m.attachments.iter().map(|a| self.render_attachment(a, cx))),
                        ),
                )
            })
    }

    /// Replaces `cid:` image references with the downloaded attachment as a `data:` URL,
    /// which the HTML view decodes inline. Images not downloaded yet stay as they are.
    fn resolve_inline_images(&self, m: &Message, html: String) -> String {
        if !html.contains("cid:") {
            return html;
        }
        let mut out = html;
        for a in m.attachments.iter().filter(|a| a.is_inline_image()) {
            let Some(cid) = a.content_id.as_deref() else { continue };
            let needle = format!("src=\"cid:{cid}\"");
            if !out.contains(&needle) {
                continue;
            }
            if let Some(url) = self.inline_image_data_url(a) {
                out = out.replace(&needle, &format!("src=\"{url}\""));
            }
        }
        out
    }

    fn inline_image_data_url(&self, a: &Attachment) -> Option<String> {
        if let Some(cached) = self.inline_images.borrow().get(&a.id) {
            return cached.clone();
        }
        let path = a.local_path.as_deref().map(std::path::PathBuf::from).filter(|p| p.exists())?;
        let bytes = std::fs::read(&path).ok()?;
        let mime = a.mime_type.split(';').next().unwrap_or("image/png").trim();
        let url = format!("data:{mime};base64,{}", base64_encode(&bytes));
        self.inline_images.borrow_mut().insert(a.id.clone(), Some(url.clone()));
        Some(url)
    }

    /// Whether quoted HTML repeats the text of an earlier message in the open conversation.
    fn quote_repeats_thread(&self, quoted_html: &str, m: &Message) -> bool {
        fn normalized(text: &str) -> String {
            text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
        }
        let quoted = normalized(&overlook_core::sanitize::html_to_text(quoted_html));
        if quoted.len() < 40 {
            return false;
        }
        self.thread.iter().filter(|other| other.id != m.id && other.date <= m.date).any(|other| {
            let text = other.body.text.clone().or_else(|| other.body.html.as_deref().map(overlook_core::sanitize::html_to_text)).unwrap_or_default();
            let text = normalized(&text);
            if text.len() < 40 {
                return false;
            }
            // The first line or two of the earlier message, found inside the quote.
            let probe: String = text.chars().take(120).collect();
            quoted.contains(probe.trim())
        })
    }

    /// Who a sender is: address book entry or correspondent, with the two things one does
    /// next: write to them, or find everything they sent.
    fn render_contact_card(&self, who: &Address, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let contact = self.store.contact(&self.account.id, &who.email).ok().flatten();
        let name = contact.as_ref().and_then(|c| c.name.clone()).or_else(|| who.name.clone()).unwrap_or_else(|| who.email.clone());
        let source = match &contact {
            Some(c) if c.in_address_book => "In your Google Contacts",
            Some(_) => "Someone you have corresponded with",
            None => "Not in your contacts",
        };
        let email = who.email.clone();
        let compose_to = who.clone();
        let search_from = format!("from:{}", who.email);
        h_flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(10.))
            .bg(p.chip)
            .border_1()
            .border_color(p.divider)
            .child(match contact.as_ref().and_then(|c| c.photo_url.clone()) {
                Some(url) => div().size(px(44.)).rounded_full().overflow_hidden().bg(p.avatar).child(img(SharedUri::from(url)).size_full().object_fit(ObjectFit::Cover)).into_any_element(),
                None => avatar(who, 44., &p).into_any_element(),
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().text_size(px(12.)).text_color(p.muted).child(email))
                    .child(div().text_size(px(11.)).text_color(p.muted).child(source)),
            )
            .child(Button::new("card-compose").small().icon(icon("pen-line").size(px(14.))).label("New message").on_click(cx.listener(move |this, _, window, cx| {
                this.compose_new(window, cx);
                if let Some(c) = this.composer.as_ref() {
                    let to = c.to.clone();
                    let header = compose_to.to_header();
                    to.update(cx, |s, cx| s.set_value(header, window, cx));
                }
                this.sync_composer_fields(cx);
            })))
            .child({
                let who = who.clone();
                let in_book = contact.as_ref().is_some_and(|c| c.in_address_book);
                Button::new("card-contacts").ghost().small().icon(icon(if in_book { "book-user" } else { "user-plus" }).size(px(14.))).label(if in_book { "Open in Contacts" } else { "Add to contacts" }).on_click(cx.listener(move |this, _, window, cx| this.show_in_contacts(who.clone(), window, cx)))
            })
            .child(Button::new("card-search").ghost().small().icon(icon("search").size(px(14.))).label("Mail from them").on_click(cx.listener(move |this, _, window, cx| {
                let input = this.search_input.clone();
                let q = search_from.clone();
                input.update(cx, |s, cx| s.set_value(q.clone(), window, cx));
                this.set_search(q, window, cx);
            })))
    }

    /// The image attachment shown over the window, with Open and Save.
    pub(super) fn render_quick_look(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(ql) = self.quick_look.clone() else { return div().into_any_element() };
        let a = ql.attachment.clone();
        let open = a.clone();
        let save = a.clone();
        let is_pdf = a.is_pdf();
        let page_count = ql.pages.len();
        let content: AnyElement = if ql.rendering {
            div().text_color(gpui_kit::white()).text_size(px(14.)).child("Rendering pages…").into_any_element()
        } else if let Some(e) = &ql.error {
            div().text_color(gpui_kit::white()).text_size(px(14.)).child(e.clone()).into_any_element()
        } else if is_pdf {
            div()
                .id("quick-look-pages")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(16.))
                .children(ql.pages.iter().enumerate().map(|(i, page)| {
                    div()
                        .w(px(900.))
                        .max_w_full()
                        .bg(gpui_kit::white())
                        .rounded(px(4.))
                        .shadow_lg()
                        .child(img(page.clone()).w_full().object_fit(ObjectFit::Contain))
                        .id(SharedString::from(format!("pdf-page-{i}")))
                }))
                .when(page_count >= crate::pdf_preview::MAX_PAGES, |d| d.child(div().text_color(gpui_kit::white()).text_size(px(12.)).opacity(0.7).child(format!("Showing the first {page_count} pages. Open the file for the rest."))))
                .into_any_element()
        } else {
            match ql.pages.first() {
                Some(path) => img(path.clone()).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(6.)).into_any_element(),
                None => div().into_any_element(),
            }
        };
        let mut backdrop: Hsla = gpui_kit::black();
        backdrop.a = 0.86;
        div()
            .id("quick-look")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .flex()
            .flex_col()
            .on_click(cx.listener(|this, _, _w, cx| {
                this.quick_look = None;
                cx.notify();
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(20.))
                    .h(px(52.))
                    .text_color(gpui_kit::white())
                    .child(icon(if is_pdf { "file-text" } else { "image" }).size(px(16.)).text_color(gpui_kit::white()))
                    .child(div().flex_1().truncate().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(a.filename.clone()))
                    .child(div().text_size(px(12.)).opacity(0.7).child(if is_pdf && page_count > 0 { format!("{} · {} page{}", a.human_size(), page_count, if page_count == 1 { "" } else { "s" }) } else { a.human_size() }))
                    .child(Button::new("ql-open").ghost().small().icon(icon("external-link").size(px(15.))).label("Open").on_click(cx.listener(move |_this, _, _w, cx| {
                        cx.stop_propagation();
                        if let Some(path) = open.local_path.as_deref() {
                            cx.open_with_system(std::path::Path::new(path));
                        }
                    })))
                    .child(Button::new("ql-save").ghost().small().icon(icon("download").size(px(15.))).label("Save as…").on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.save_attachment(save.clone(), window, cx);
                    })))
                    .child(Button::new("ql-close").ghost().small().icon(icon("x").size(px(16.))).tooltip("Close (Esc)").on_click(cx.listener(|this, _, _w, cx| {
                        this.quick_look = None;
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .into_any_element()
    }

    fn render_attachment(&self, a: &Attachment, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let downloading = self.downloading.contains(&a.id);
        let open = a.clone();
        let save = a.clone();
        let thumb = if a.is_image() {
            a.local_path.as_deref().map(std::path::PathBuf::from).filter(|p| p.exists())
        } else {
            self.pdf_thumbs.get(&a.id).cloned()
        };
        attachment_card(a, &p, downloading, thumb)
            .id(SharedString::from(format!("att-{}", a.id)))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(
                Button::new(SharedString::from(format!("save-{}", a.id)))
                    .ghost()
                    .xsmall()
                    .icon(icon("download").size(px(14.)))
                    .tooltip("Save as…")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.save_attachment(save.clone(), window, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.open_attachment(open.clone(), window, cx)))
    }

    /// Invitation card for messages that carry a `text/calendar` part.
    fn render_invitation_card(&self, inv: &overlook_core::ics::Invitation, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let event = self.store.event_by_ical_uid(&self.account.id, &inv.uid).ok().flatten();
        let title = event.as_ref().map(|e| e.display_title()).or_else(|| inv.summary.clone()).unwrap_or_else(|| "Invitation".into());
        let when = match &event {
            Some(e) if e.all_day => format!("{} · All day", overlook_core::tz::format_day(e.start)),
            Some(e) => format!("{} · {} – {}", overlook_core::tz::zoned(e.start).strftime("%A, %B %-d"), overlook_core::tz::format_clock(e.start), overlook_core::tz::format_clock(e.end)),
            None => match inv.start {
                Some(s) if inv.all_day => format!("{} · All day", overlook_core::tz::format_day(s)),
                Some(s) => format!("{} · {}", overlook_core::tz::zoned(s).strftime("%A, %B %-d"), overlook_core::tz::format_clock(s)),
                None => String::new(),
            },
        };
        let my = event.as_ref().and_then(|e| e.my_response());
        let status_line = if inv.is_cancellation() {
            Some(("This event was cancelled.".to_owned(), p.error))
        } else if event.is_none() {
            Some(("Not on your calendar yet. It will appear after the next calendar sync.".to_owned(), p.muted))
        } else {
            my.filter(|r| *r != ResponseStatus::NeedsAction).map(|r| (format!("You {} this invitation.", r.label().to_lowercase()), p.success))
        };
        let event_id = event.as_ref().map(|e| e.id.clone());
        let can_respond = !inv.is_cancellation() && event.as_ref().is_some_and(|e| e.is_invitation());
        let response_button = |label: &'static str, response: ResponseStatus, icon_name: &'static str, cx: &mut Context<Self>| {
            let id = event_id.clone();
            Button::new(SharedString::from(format!("inv-{label}")))
                .small()
                .icon(icon(icon_name).size(px(14.)))
                .label(label)
                .when(my == Some(response), |b| b.primary())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(id) = id.clone() {
                        this.respond_to_event(id, response, window, cx);
                    }
                }))
        };
        v_flex()
            .gap(px(10.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.divider)
            .bg(p.chip)
            .child(
                h_flex()
                    .items_start()
                    .gap(px(10.))
                    .child(icon("calendar").size(px(18.)).text_color(p.accent))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title))
                            .when(!when.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(p.muted).child(when)))
                            .children(event.as_ref().and_then(|e| e.location.clone()).map(|l| div().text_size(px(12.)).text_color(p.muted).child(l))),
                    ),
            )
            .children(status_line.map(|(text, color)| div().text_size(px(12.)).text_color(color).child(text)))
            .child(
                h_flex()
                    .gap(px(6.))
                    .when(can_respond, |r| {
                        r.child(response_button("Accept", ResponseStatus::Accepted, "check", cx))
                            .child(response_button("Tentative", ResponseStatus::Tentative, "circle-question-mark", cx))
                            .child(response_button("Decline", ResponseStatus::Declined, "x", cx))
                    })
                    .children(event_id.clone().map(|id| {
                        Button::new("show-in-calendar")
                            .ghost()
                            .small()
                            .icon(icon("calendar-days").size(px(14.)))
                            .label("Show in calendar")
                            .on_click(cx.listener(move |this, _, window, cx| this.show_event_in_calendar(id.clone(), window, cx)))
                    })),
            )
    }

    fn render_outbox_item(&self, d: &Draft, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let state = overlook_core::status::draft_state_label(&d.state);
        let id = d.id.clone();
        let cancellable = matches!(d.state, DraftState::Queued | DraftState::Failed(_));
        v_flex()
            .size_full()
            .px(px(28.))
            .pt(px(22.))
            .gap(px(14.))
            .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child(d.display_subject()))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("To {}", d.to.iter().map(Address::to_header).collect::<Vec<_>>().join(", "))),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(8.))
                    .bg(p.chip)
                    .text_size(px(13.))
                    .child(icon("clock").size(px(15.)).text_color(p.accent))
                    .child(state)
                    .child(div().flex_1())
                    .when(cancellable, |row| {
                        row.child(
                            Button::new("outbox-edit")
                                .small()
                                .label("Edit message")
                                .on_click(cx.listener(move |this, _, window, cx| this.cancel_send(id.clone(), window, cx))),
                        )
                    }),
            )
            .child(divider(&p))
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .max_w(px(760.))
                    .child({
                        let source = d.in_reply_to.as_ref().and_then(|id| self.store.message(id).ok().flatten());
                        let html = overlook_core::compose::outgoing_bodies(d, source.as_ref()).html;
                        TextView::html(SharedString::from(format!("outbox-{}", d.id)), html).selectable(true)
                    }),
            )
    }
}

fn toolbar_button(
    icon_name: &'static str,
    label: &'static str,
    tooltip: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(SharedString::from(format!("tb-{icon_name}-{label}")))
        .ghost()
        .small()
        .icon(icon(icon_name).size(px(16.)))
        .label(label)
        .tooltip(tooltip)
        .on_click(on_click)
}

fn attachment_card(a: &Attachment, p: &crate::theme::Palette, downloading: bool, thumbnail: Option<std::path::PathBuf>) -> Div {
    let badge_bg = match a.badge().as_str() {
        "PDF" => (c(0xFBE9E7, p), c(0xA13E37, p)),
        "FIG" | "PNG" | "JPG" | "SVG" => (c(0xEEE8FA, p), c(0x6E55B7, p)),
        _ => (p.chip, p.muted),
    };
    h_flex()
        .items_center()
        .gap(px(12.))
        .w(px(300.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(p.divider)
        .bg(p.surface)
        .child(match thumbnail {
            Some(path) => div().flex_none().size(px(36.)).rounded(px(6.)).overflow_hidden().bg(p.chip).child(img(path).size_full().object_fit(ObjectFit::Cover)),
            None => div()
                .flex_none()
                .size(px(36.))
                .rounded(px(6.))
                .bg(badge_bg.0)
                .text_color(badge_bg.1)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(a.badge()),
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(a.filename.clone()))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(if downloading {
                            format!("{} · Downloading…", a.human_size())
                        } else if a.is_downloaded() {
                            format!("{} · {}", a.human_size(), if a.is_image() || (a.is_pdf() && crate::pdf_preview::available()) { "Preview" } else { "Open" })
                        } else {
                            format!("{} · Click to download", a.human_size())
                        }),
                ),
        )
}

fn c(hex: u32, p: &crate::theme::Palette) -> Hsla {
    if p.is_dark() {
        // Darken pastel badges for the dark theme so they do not glow.
        let mut h: Hsla = Rgba::from(gpui_kit::rgb(hex)).into();
        h.l = (h.l * 0.35).min(0.3);
        h.s *= 0.6;
        h
    } else {
        Rgba::from(gpui_kit::rgb(hex)).into()
    }
}

/// Standard base64 with padding, enough for `data:` URLs.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}
