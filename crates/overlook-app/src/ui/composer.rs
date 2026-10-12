//! Composer: new message, reply, and forward, with durable autosave to the local store.

use super::widgets::*;
use super::MailWindow;
use crate::assets::icon;
use crate::theme::ActivePalette;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::{h_flex, v_flex};
use overlook_core::status::draft_state_label;
use overlook_core::time::{format_clock, now};
use overlook_core::*;
use std::time::Duration;

pub struct Composer {
    pub draft: Draft,
    pub to: Entity<InputState>,
    pub cc: Entity<InputState>,
    pub bcc: Entity<InputState>,
    pub subject: Entity<InputState>,
    pub body: Entity<TextareaState>,
    pub show_cc: bool,
    pub show_bcc: bool,
    pub dirty: bool,
    pub last_saved: Option<i64>,
    pub save_generation: u64,
    pub suggestions: Vec<Address>,
    pub error: Option<String>,
    pub _subscriptions: Vec<Subscription>,
}

impl MailWindow {
    /// Opens the composer for `draft`, creating the input entities and autosave wiring.
    pub(super) fn open_composer(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.close_composer(window, cx);
        let join = |list: &[Address]| list.iter().map(Address::to_header).collect::<Vec<_>>().join(", ");
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("Recipients").default_value(join(&draft.to)));
        let cc = cx.new(|cx| InputState::new(window, cx).placeholder("Cc").default_value(join(&draft.cc)));
        let bcc = cx.new(|cx| InputState::new(window, cx).placeholder("Bcc").default_value(join(&draft.bcc)));
        let subject = cx.new(|cx| InputState::new(window, cx).placeholder("Add a subject").default_value(draft.subject.clone()));
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder("Write your message…").default_value(draft.body_text.clone()));

        let mut subs = Vec::new();
        for state in [&to, &cc, &bcc, &subject] {
            subs.push(cx.subscribe_in(state, window, |this, _state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.composer_changed(window, cx),
                // Ctrl+Enter sends from any field; plain Enter in a header field moves on.
                InputEvent::PressEnter { secondary: true, .. } => this.send_composer(window, cx),
                InputEvent::PressEnter { .. } => this.focus_next_composer_field(window, cx),
                _ => {}
            }));
        }
        subs.push(cx.subscribe_in(&body, window, |this, _state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.composer_changed(window, cx),
            InputEvent::PressEnter { secondary: true, .. } => this.send_composer(window, cx),
            _ => {}
        }));

        let focus_to = draft.to.is_empty();
        let show_cc = !draft.cc.is_empty();
        let show_bcc = !draft.bcc.is_empty();
        self.composer = Some(Composer {
            draft,
            to: to.clone(),
            cc,
            bcc,
            subject,
            body: body.clone(),
            show_cc,
            show_bcc,
            dirty: false,
            last_saved: None,
            save_generation: 0,
            suggestions: Vec::new(),
            error: None,
            _subscriptions: subs,
        });
        if focus_to {
            to.update(cx, |s, cx| s.focus(window, cx));
        } else {
            body.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// Enter in To/Cc/Bcc/Subject moves focus to the next field, ending in the body.
    fn focus_next_composer_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.composer.as_ref() else { return };
        let order: Vec<Entity<InputState>> = {
            let mut v = vec![c.to.clone()];
            if c.show_cc { v.push(c.cc.clone()); }
            if c.show_bcc { v.push(c.bcc.clone()); }
            v.push(c.subject.clone());
            v
        };
        let focused = order.iter().position(|s| s.read(cx).focus_handle(cx).is_focused(window));
        match focused {
            Some(i) if i + 1 < order.len() => {
                let next = order[i + 1].clone();
                next.update(cx, |s, cx| s.focus(window, cx));
            }
            _ => {
                let body = c.body.clone();
                body.update(cx, |s, cx| s.focus(window, cx));
            }
        }
    }

    /// Reads the inputs back into the draft.
    pub(super) fn sync_composer_fields(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.composer.as_mut() else { return };
        c.draft.to = Address::parse_list(&c.to.read(cx).value());
        c.draft.cc = Address::parse_list(&c.cc.read(cx).value());
        c.draft.bcc = Address::parse_list(&c.bcc.read(cx).value());
        c.draft.subject = c.subject.read(cx).value().trim().to_owned();
        c.draft.body_text = c.body.read(cx).value().to_string();
        c.draft.updated_at = now();
    }

    fn composer_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_composer_fields(cx);
        let Some(c) = self.composer.as_mut() else { return };
        c.dirty = true;
        c.error = None;
        c.save_generation += 1;
        let generation = c.save_generation;

        // Recipient suggestions from previous correspondents, keyed on the last token.
        let to_value = c.to.read(cx).value().to_string();
        let focused = c.to.read(cx).focus_handle(cx).is_focused(window);
        let last_token = to_value.rsplit([',', ';']).next().unwrap_or("").trim().to_owned();
        c.suggestions = if focused && last_token.len() >= 1 {
            self.store
                .known_recipients(&self.account.id, &last_token, 5)
                .unwrap_or_default()
                .into_iter()
                .filter(|a| !a.email.eq_ignore_ascii_case(&self.account.email))
                .filter(|a| !last_token.eq_ignore_ascii_case(&a.email) && !last_token.eq_ignore_ascii_case(&a.to_header()))
                .collect()
        } else {
            Vec::new()
        };
        cx.notify();

        // Debounced durable autosave.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.composer.as_ref().is_some_and(|c| c.save_generation == generation && c.dirty) {
                    this.save_composer(window, cx);
                }
            });
        })
        .detach();
    }

    /// Writes the draft to the store. Only after this succeeds does the UI say it is saved.
    pub(super) fn save_composer(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.sync_composer_fields(cx);
        let Some(c) = self.composer.as_mut() else { return false };
        if c.draft.is_empty() && c.last_saved.is_none() {
            return false;
        }
        match self.store.save_draft(&c.draft) {
            Ok(()) => {
                c.dirty = false;
                c.last_saved = Some(now());
                self.reload_drafts();
                cx.notify();
                true
            }
            Err(e) => {
                c.error = Some(format!("Couldn’t save on this device: {e}"));
                cx.notify();
                false
            }
        }
    }

    pub(super) fn close_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            self.save_composer(window, cx);
            if let Some(c) = self.composer.take() {
                if c.draft.is_empty() && c.last_saved.is_some() {
                    // Nothing was written; do not leave an empty draft behind.
                    let _ = self.store.delete_draft(&c.draft.id);
                    self.reload_drafts();
                }
            }
            self.list_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn discard_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = self.composer.take() {
            let _ = self.store.delete_draft(&c.draft.id);
            self.reload_drafts();
            window.push_notification(Notification::info("Draft discarded."), cx);
            self.list_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn send_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_composer_fields(cx);
        let Some(c) = self.composer.as_mut() else { return };
        if let Err(e) = c.draft.validate_for_send() {
            c.error = Some(e.to_string());
            let to = c.to.clone();
            to.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
            return;
        }
        if c.draft.body_text.trim().is_empty() && c.draft.subject.trim().is_empty() {
            c.error = Some("Add a subject or a message before sending.".into());
            cx.notify();
            return;
        }
        let draft = c.draft.clone();
        match self.store.queue_send(&draft) {
            Ok(_) => {
                self.composer = None;
                self.reload(cx);
                let online = self.connected(&self.account.id);
                window.push_notification(
                    if online {
                        Notification::info("Sending…")
                    } else {
                        Notification::info("Waiting for connection. We’ll send this when you’re back online.")
                    },
                    cx,
                );
                self.list_focus.focus(window, cx);
                if online {
                    self.sync_now(window, cx);
                }
            }
            Err(e) => {
                if let Some(c) = self.composer.as_mut() {
                    c.error = Some(e.to_string());
                }
            }
        }
        cx.notify();
    }

    pub(super) fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.composer.as_ref() else { return };
        let draft_id = c.draft.id.clone();
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        let outgoing_dir = self.outgoing_dir.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let mut added = Vec::new();
            for path in paths {
                let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
                let dir = outgoing_dir.join(draft_id.as_str());
                if std::fs::create_dir_all(&dir).is_err() {
                    continue;
                }
                let dest = dir.join(&name);
                if std::fs::copy(&path, &dest).is_err() {
                    continue;
                }
                let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                added.push(DraftAttachment {
                    id: AttachmentId::generate(),
                    filename: name,
                    mime_type: "application/octet-stream".into(),
                    size_bytes: size,
                    local_path: dest.to_string_lossy().to_string(),
                });
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(c) = this.composer.as_mut() {
                    if c.draft.id == draft_id {
                        c.draft.attachments.extend(added);
                        c.dirty = true;
                        this.save_composer(window, cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let Some(c) = self.composer.as_ref() else { return div().into_any_element() };
        let title = match c.draft.kind {
            DraftKind::New => "New message",
            DraftKind::Reply => "Reply",
            DraftKind::ReplyAll => "Reply all",
            DraftKind::Forward => "Forward",
        };
        let inline = c.draft.kind != DraftKind::New;
        let saved_text = match (c.dirty, c.last_saved) {
            (true, _) => "Saving…".to_owned(),
            (false, Some(ts)) => {
                let (_, time, _) = overlook_core::time::civil(ts, self.utc_offset);
                format!("{} · {}", draft_state_label(&c.draft.state), format_clock(time))
            }
            (false, None) => "Not saved yet".to_owned(),
        };
        let owner = self.owner();

        let field_label = |text: &'static str| {
            div().w(px(56.)).flex_none().text_size(px(12.)).text_color(p.muted).child(text)
        };
        let field_row = |label: &'static str, input: AnyElement| {
            h_flex()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .min_h(px(40.))
                .border_b_1()
                .border_color(p.divider)
                .child(field_label(label))
                .child(div().flex_1().min_w_0().child(input))
        };

        let suggestions: Option<Div> = (!c.suggestions.is_empty()).then(|| {
            h_flex()
                .flex_wrap()
                .gap(px(6.))
                .px(px(80.))
                .py(px(6.))
                .border_b_1()
                .border_color(p.divider)
                .children(c.suggestions.iter().cloned().map(|a| {
                    let header = a.to_header();
                    Button::new(SharedString::from(format!("suggest-{}", a.email)))
                        .xsmall()
                        .label(if a.name.is_some() { format!("{} · {}", a.display_name(), a.email) } else { a.email.clone() })
                        .on_click(cx.listener(move |this, _, window, cx| this.accept_suggestion(header.clone(), window, cx)))
                }))
        });

        let attachments: Option<Div> = (!c.draft.attachments.is_empty()).then(|| {
            h_flex()
                .flex_wrap()
                .gap(px(8.))
                .px(px(16.))
                .py(px(8.))
                .children(c.draft.attachments.iter().map(|a| {
                    let id = a.id.clone();
                    h_flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(p.divider)
                        .text_size(px(12.))
                        .child(icon("paperclip").size(px(13.)).text_color(p.muted))
                        .child(a.filename.clone())
                        .child(div().text_color(p.muted).child(overlook_core::human_size(a.size_bytes)))
                        .child(
                            Button::new(SharedString::from(format!("rm-att-{}", a.id)))
                                .ghost()
                                .xsmall()
                                .icon(icon("x").size(px(12.)))
                                .on_click(cx.listener(move |this, _, window, cx| this.remove_attachment(id.clone(), window, cx))),
                        )
                }))
        });

        let error = c.error.clone().map(|e| {
            h_flex()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
                .text_color(p.error)
                .child(icon("circle-alert").size(px(14.)).text_color(p.error))
                .child(e)
        });

        v_flex()
            .when(inline, |d| d.mx(px(28.)).mt(px(18.)).mb(px(28.)).rounded(px(10.)).border_1().border_color(p.divider))
            .when(!inline, |d| d.size_full())
            .bg(p.surface)
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .px(px(16.))
                    .h(px(46.))
                    .border_b_1()
                    .border_color(p.divider)
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(5.))
                            .text_size(px(11.))
                            .text_color(if c.dirty { p.muted } else { p.success })
                            .child(icon(if c.dirty { "loader-circle" } else { "check" }).size(px(13.)))
                            .child(saved_text),
                    )
                    .child(div().w(px(8.)))
                    .child(
                        Button::new("close-composer")
                            .ghost()
                            .small()
                            .icon(icon("x").size(px(16.)))
                            .tooltip("Save and close (Esc)")
                            .on_click(cx.listener(|this, _, window, cx| this.close_composer(window, cx))),
                    ),
            )
            .child(field_row(
                "From",
                div().text_size(px(13.)).child(format!("{} <{}>", owner.display_name(), owner.email)).into_any_element(),
            ))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .min_h(px(40.))
                    .border_b_1()
                    .border_color(p.divider)
                    .child(field_label("To"))
                    .child(div().flex_1().min_w_0().child(Input::new(&c.to).appearance(false)))
                    .when(!c.show_cc, |d| {
                        d.child(Button::new("show-cc").ghost().xsmall().label("Cc").on_click(cx.listener(|this, _, _w, cx| {
                            if let Some(c) = this.composer.as_mut() { c.show_cc = true; }
                            cx.notify();
                        })))
                    })
                    .when(!c.show_bcc, |d| {
                        d.child(Button::new("show-bcc").ghost().xsmall().label("Bcc").on_click(cx.listener(|this, _, _w, cx| {
                            if let Some(c) = this.composer.as_mut() { c.show_bcc = true; }
                            cx.notify();
                        })))
                    }),
            )
            .children(suggestions)
            .when(c.show_cc, |d| d.child(field_row("Cc", Input::new(&c.cc).appearance(false).into_any_element())))
            .when(c.show_bcc, |d| d.child(field_row("Bcc", Input::new(&c.bcc).appearance(false).into_any_element())))
            .child(field_row("Subject", Input::new(&c.subject).appearance(false).into_any_element()))
            .child(
                div()
                    .flex_1()
                    .min_h(px(if inline { 180. } else { 240. }))
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(14.))
                    .child(Textarea::new(&c.body).appearance(false).bordered(false).h_full()),
            )
            .children(attachments)
            .children(error)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(p.divider)
                    .child(
                        Button::new("send")
                            .primary()
                            .label("Send")
                            .icon(icon("send").size(px(15.)))
                            .tooltip("Send (Ctrl Enter)")
                            .on_click(cx.listener(|this, _, window, cx| this.send_composer(window, cx))),
                    )
                    .child(
                        Button::new("attach")
                            .ghost()
                            .small()
                            .icon(icon("paperclip").size(px(16.)))
                            .tooltip("Attach files")
                            .on_click(cx.listener(|this, _, window, cx| this.attach_files(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(kbd("Ctrl Enter", &p))
                    .child(
                        Button::new("discard")
                            .ghost()
                            .small()
                            .icon(icon("trash").size(px(16.)).text_color(p.error))
                            .tooltip("Discard draft")
                            .on_click(cx.listener(|this, _, window, cx| this.discard_composer(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn accept_suggestion(&mut self, header: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.composer.as_ref() else { return };
        let current = c.to.read(cx).value().to_string();
        let mut parts: Vec<String> = current.split([',', ';']).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        // The last token is the partial text being completed.
        if !current.trim_end().ends_with([',', ';']) {
            parts.pop();
        }
        parts.push(header);
        let value = format!("{}, ", parts.join(", "));
        let to = c.to.clone();
        to.update(cx, |s, cx| {
            s.set_value(value, window, cx);
            s.focus(window, cx);
        });
        if let Some(c) = self.composer.as_mut() {
            c.suggestions.clear();
        }
        self.composer_changed(window, cx);
    }

    fn remove_attachment(&mut self, id: AttachmentId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = self.composer.as_mut() {
            if let Some(pos) = c.draft.attachments.iter().position(|a| a.id == id) {
                let att = c.draft.attachments.remove(pos);
                let _ = std::fs::remove_file(&att.local_path);
                c.dirty = true;
            }
        }
        self.save_composer(window, cx);
    }
}
