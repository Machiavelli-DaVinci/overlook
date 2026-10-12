//! Settings dialog: Accounts (with signatures), General, Mail, Calendar, About.
//!
//! Rendered as an overlay in the main window. Every control writes its preference
//! immediately and the window applies it; nothing here is decorative.

use super::widgets::*;
use super::MailWindow;
use crate::assets::icon;
use crate::theme::ActivePalette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::*;
use overlook_core::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    Accounts,
    General,
    Mail,
    Calendar,
    About,
}

impl SettingsPage {
    const ALL: [SettingsPage; 5] = [SettingsPage::Accounts, SettingsPage::General, SettingsPage::Mail, SettingsPage::Calendar, SettingsPage::About];

    fn label(self) -> &'static str {
        match self {
            SettingsPage::Accounts => "Accounts",
            SettingsPage::General => "General",
            SettingsPage::Mail => "Mail",
            SettingsPage::Calendar => "Calendar",
            SettingsPage::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            SettingsPage::Accounts => "user-round",
            SettingsPage::General => "settings-2",
            SettingsPage::Mail => "mail",
            SettingsPage::Calendar => "calendar",
            SettingsPage::About => "info",
        }
    }
}

pub struct SettingsState {
    pub page: SettingsPage,
    /// Account shown on the Accounts page.
    pub account: AccountId,
    pub name_input: Entity<InputState>,
    pub signature: Entity<TextareaState>,
    pub attachments_dir: Entity<InputState>,
    pub work_start: Entity<InputState>,
    pub work_end: Entity<InputState>,
    pub _subscriptions: Vec<Subscription>,
}

impl MailWindow {
    pub(crate) fn save_prefs(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.store.set_setting("prefs", &self.prefs) {
            tracing::warn!("could not save preferences: {e}");
        }
        overlook_core::tz::set_clock_24h(self.prefs.clock_24h);
        self.reminders_enabled = self.prefs.event_reminders;
        self.reload(cx);
    }

    pub(crate) fn account_prefs_for(&self, id: &AccountId) -> AccountPrefs {
        self.account_prefs.get(id).cloned().unwrap_or_default()
    }

    pub(crate) fn save_account_prefs(&mut self, id: &AccountId, prefs: AccountPrefs) {
        if let Err(e) = self.store.set_setting(&format!("account_prefs:{}", id.0), &prefs) {
            tracing::warn!("could not save account preferences: {e}");
        }
        self.account_prefs.insert(id.clone(), prefs);
    }

    pub(crate) fn open_settings(&mut self, page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.account.id.clone();
        self.build_settings(page, account, window, cx);
    }

    fn build_settings(&mut self, page: SettingsPage, account: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let acct = self.accounts.iter().find(|a| a.id == account).cloned().unwrap_or_else(|| self.account.clone());
        let ap = self.account_prefs_for(&acct.id);
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Your name").default_value(acct.display_name.clone()));
        let signature = cx.new(|cx| TextareaState::new(window, cx).placeholder("Jason Alexander\nLast Refuge").default_value(ap.signature.clone()));
        let attachments_dir = cx.new(|cx| InputState::new(window, cx).placeholder("~/Downloads").default_value(self.prefs.attachments_dir.clone().unwrap_or_default()));
        let work_start = cx.new(|cx| InputState::new(window, cx).placeholder("8:00 AM").default_value(hour_text(self.prefs.work_start_hour)));
        let work_end = cx.new(|cx| InputState::new(window, cx).placeholder("6:00 PM").default_value(hour_text(self.prefs.work_end_hour)));

        let mut subs = Vec::new();
        let for_account = acct.id.clone();
        subs.push(cx.subscribe_in(&name_input, window, move |this, state, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                let name = state.read(cx).value().trim().to_owned();
                if name.is_empty() {
                    return;
                }
                if let Some(a) = this.accounts.iter_mut().find(|a| a.id == for_account) {
                    a.display_name = name.clone();
                    let _ = this.store.upsert_account(a);
                }
                if this.account.id == for_account {
                    this.account.display_name = name;
                }
                cx.notify();
            }
        }));
        let for_account = acct.id.clone();
        subs.push(cx.subscribe_in(&signature, window, move |this, state, event: &InputEvent, _window, _cx| {
            if matches!(event, InputEvent::Change) {
                let mut p = this.account_prefs_for(&for_account);
                p.signature = state.read(_cx).value().to_string();
                this.save_account_prefs(&for_account, p);
            }
        }));
        subs.push(cx.subscribe_in(&attachments_dir, window, |this, state, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                let v = state.read(cx).value().trim().to_owned();
                this.prefs.attachments_dir = if v.is_empty() { None } else { Some(v) };
                this.save_prefs(cx);
            }
        }));
        subs.push(cx.subscribe_in(&work_start, window, |this, state, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(t) = overlook_core::tz::parse_clock(&state.read(cx).value()) {
                    this.prefs.work_start_hour = t.hour() as u8;
                    this.save_prefs(cx);
                }
            }
        }));
        subs.push(cx.subscribe_in(&work_end, window, |this, state, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(t) = overlook_core::tz::parse_clock(&state.read(cx).value()) {
                    this.prefs.work_end_hour = t.hour() as u8;
                    this.save_prefs(cx);
                }
            }
        }));

        self.settings = Some(SettingsState { page, account: acct.id, name_input, signature, attachments_dir, work_start, work_end, _subscriptions: subs });
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = None;
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    fn set_settings_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        if let Some(s) = self.settings.as_mut() {
            s.page = page;
        }
        cx.notify();
    }

    fn set_settings_account(&mut self, id: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        self.build_settings(SettingsPage::Accounts, id, window, cx);
    }

    // ---------------------------------------------------------------- render

    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = *cx.palette();
        let Some(state) = self.settings.as_ref() else { return div().into_any_element() };
        let page = state.page;

        let nav = v_flex()
            .w(px(200.))
            .flex_none()
            .h_full()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.divider)
            .p(px(12.))
            .gap(px(2.))
            .child(div().px(px(10.)).pt(px(6.)).pb(px(14.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Settings"))
            .children(SettingsPage::ALL.iter().map(|&pg| {
                let active = pg == page;
                h_flex()
                    .id(SharedString::from(format!("settings-nav-{}", pg.label())))
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .h(px(34.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(if active { p.accent } else { p.foreground })
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .when(active, |d| d.bg(p.selection))
                    .when(!active, |d| d.hover(|d| d.bg(p.hover)))
                    .child(icon(pg.icon()).size(px(16.)).text_color(if active { p.accent } else { p.muted }))
                    .child(pg.label())
                    .on_click(cx.listener(move |this, _, _w, cx| this.set_settings_page(pg, cx)))
            }));

        let content: AnyElement = match page {
            SettingsPage::Accounts => self.render_settings_accounts(state, cx).into_any_element(),
            SettingsPage::General => self.render_settings_general(cx).into_any_element(),
            SettingsPage::Mail => self.render_settings_mail(state, cx).into_any_element(),
            SettingsPage::Calendar => self.render_settings_calendar(state, cx).into_any_element(),
            SettingsPage::About => self.render_settings_about(cx).into_any_element(),
        };

        div()
            .id("settings-overlay")
            .absolute()
            .inset_0()
            .bg(gpui_kit::black().opacity(if p.is_dark() { 0.55 } else { 0.3 }))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
            .child(
                h_flex()
                    .id("settings-dialog")
                    .w(px(960.))
                    .h(px(640.))
                    .max_w(relative(0.94))
                    .max_h(relative(0.92))
                    .rounded(px(12.))
                    .overflow_hidden()
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.divider)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(nav)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                h_flex()
                                    .items_center()
                                    .h(px(52.))
                                    .px(px(24.))
                                    .border_b_1()
                                    .border_color(p.divider)
                                    .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(page.label()))
                                    .child(kbd("Esc", &p))
                                    .child(Button::new("settings-close").ghost().small().icon(icon("x").size(px(16.))).ml(px(8.)).on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))),
                            )
                            .child(v_flex().id("settings-content").flex_1().min_h_0().p(px(24.)).gap(px(22.)).child(content).overflow_y_scrollbar()),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_accounts(&self, state: &SettingsState, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let selected = state.account.clone();
        let account = self.accounts.iter().find(|a| a.id == selected).cloned().unwrap_or_else(|| self.account.clone());
        let ap = self.account_prefs_for(&account.id);
        let status = self.store.account_status(&account.id).unwrap_or_default();
        let issue = self.provider_issues.get(&account.id).cloned();
        let email = account.email.clone();
        let is_gmail = account.provider == ProviderKind::Gmail;

        let account_list = v_flex().gap(px(4.)).children(self.accounts.iter().map(|a| {
            let id = a.id.clone();
            let active = a.id == selected;
            h_flex()
                .id(SharedString::from(format!("settings-account-{}", a.id)))
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(8.))
                .cursor_pointer()
                .border_1()
                .border_color(if active { p.accent } else { p.divider })
                .when(active, |d| d.bg(p.selection))
                .when(!active, |d| d.hover(|d| d.bg(p.hover)))
                .child(owner_avatar(&Address::named(&a.display_name, &a.email), 28., &p))
                .child(v_flex().flex_1().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).truncate().child(a.display_name.clone())).child(div().text_size(px(11.)).text_color(p.muted).truncate().child(format!("{} · {}", a.email, a.provider.label()))))
                .on_click(cx.listener(move |this, _, window, cx| this.set_settings_account(id.clone(), window, cx)))
        }));

        let remove_id = account.id.clone();
        v_flex()
            .gap(px(22.))
            .child(section("Your accounts", Some("Mail and calendar accounts on this device.")))
            .child(account_list)
            .child(Button::new("settings-add-gmail").small().icon(icon("plus").size(px(14.))).label("Add Gmail account").on_click(cx.listener(|this, _, window, cx| this.start_google_sign_in(None, window, cx))))
            .child(section(&format!("{}", account.email), Some(if is_gmail { "Connected through the Gmail and Google Calendar APIs." } else { "Sample account with fictional mail; nothing leaves this device." })))
            .child(setting_row("Your name", "Shown as the sender on messages you write.", Input::new(&state.name_input).w(px(280.)).into_any_element()))
            .child(setting_row(
                "Status",
                &status.state.sentence(status.pending_operations),
                h_flex()
                    .gap(px(6.))
                    .when(issue.is_some(), |r| {
                        let e = email.clone();
                        r.child(Button::new("settings-signin").small().label("Sign in again").on_click(cx.listener(move |this, _, window, cx| this.start_google_sign_in(Some(e.clone()), window, cx))))
                    })
                    .child(Button::new("settings-sync").ghost().small().icon(icon("refresh-cw").size(px(14.))).label("Sync now").on_click(cx.listener(|this, _, window, cx| this.sync_now(window, cx))))
                    .into_any_element(),
            ))
            .child(section("Signature", Some("Added to the end of your messages. Plain text for now; formatting arrives with the rich-text composer.")))
            .child(div().rounded(px(8.)).border_1().border_color(p.divider).h(px(140.)).child(Textarea::new(&state.signature).h_full()))
            .child(setting_row(
                "Include in new messages",
                "",
                Switch::new("sign-new").checked(ap.sign_new_messages).on_click(cx.listener({
                    let id = account.id.clone();
                    move |this, checked: &bool, _w, cx| {
                        let mut p = this.account_prefs_for(&id);
                        p.sign_new_messages = *checked;
                        this.save_account_prefs(&id, p);
                        cx.notify();
                    }
                })).into_any_element(),
            ))
            .child(setting_row(
                "Include in replies and forwards",
                "",
                Switch::new("sign-replies").checked(ap.sign_replies).on_click(cx.listener({
                    let id = account.id.clone();
                    move |this, checked: &bool, _w, cx| {
                        let mut p = this.account_prefs_for(&id);
                        p.sign_replies = *checked;
                        this.save_account_prefs(&id, p);
                        cx.notify();
                    }
                })).into_any_element(),
            ))
            .child(section("Remove", None))
            .child(setting_row(
                "Remove this account from this device",
                "Deletes downloaded mail, events, drafts, and stored sign-in for this account. Nothing is deleted at the provider.",
                Button::new("settings-remove").danger().small().label("Remove account").on_click(cx.listener(move |this, _, window, cx| {
                    this.close_settings(window, cx);
                    this.remove_account(remove_id.clone(), window, cx);
                })).into_any_element(),
            ))
    }

    fn render_settings_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = self.prefs.clone();
        let appearance = self.appearance;
        v_flex()
            .gap(px(22.))
            .child(section("Appearance", None))
            .child(setting_row(
                "Theme",
                "Follow system picks Light or Dark from your desktop setting.",
                choice(
                    &[("Light", AppearancePreference::Light), ("Dark", AppearancePreference::Dark), ("Follow system", AppearancePreference::FollowSystem)],
                    appearance,
                    cx,
                    |this, v, window, cx| this.set_appearance(v, window, cx),
                ),
            ))
            .child(setting_row(
                "Density",
                "Compact fits more messages on screen.",
                choice(&[("Comfortable", Density::Comfortable), ("Compact", Density::Compact)], self.density, cx, |this, v, _w, cx| {
                    this.density = v;
                    let _ = this.store.set_setting("density", &v);
                    cx.notify();
                }),
            ))
            .child(setting_row(
                "Reading pane",
                "",
                choice(&[("On the right", ReadingPane::Right), ("Below the list", ReadingPane::Bottom)], self.reading_pane, cx, |this, v, _w, cx| this.set_reading_pane(v, cx)),
            ))
            .child(section("Start-up and time", None))
            .child(setting_row(
                "Open Overlook in",
                "",
                choice(&[("Mail", StartupModule::Mail), ("Calendar", StartupModule::Calendar), ("Last used", StartupModule::LastUsed)], prefs.startup_module, cx, |this, v, _w, cx| {
                    this.prefs.startup_module = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Time format",
                "",
                choice(&[("12-hour", false), ("24-hour", true)], prefs.clock_24h, cx, |this, v, _w, cx| {
                    this.prefs.clock_24h = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Week starts on",
                "",
                choice(&[("Monday", true), ("Sunday", false)], prefs.week_starts_monday, cx, |this, v, _w, cx| {
                    this.prefs.week_starts_monday = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(section("Notifications", None))
            .child(setting_row(
                "New mail",
                "Desktop notification when new mail arrives.",
                choice(
                    &[("Off", NewMailNotifications::Off), ("Focused only", NewMailNotifications::FocusedOnly), ("All mail", NewMailNotifications::All)],
                    prefs.new_mail_notifications,
                    cx,
                    |this, v, _w, cx| {
                        this.prefs.new_mail_notifications = v;
                        this.save_prefs(cx);
                    },
                ),
            ))
            .child(setting_row(
                "Event reminders",
                "Desktop notification before events start.",
                Switch::new("pref-reminders").checked(prefs.event_reminders).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.event_reminders = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(section("Synchronization", None))
            .child(setting_row(
                "Check for changes every",
                "Changes you make are sent immediately; this is how often Overlook looks for new mail and events.",
                choice(&[("30 seconds", 30u32), ("1 minute", 60), ("5 minutes", 300), ("15 minutes", 900)], prefs.sync_interval_secs, cx, |this, v, _w, cx| {
                    this.prefs.sync_interval_secs = v;
                    this.save_prefs(cx);
                }),
            ))
    }

    fn render_settings_mail(&self, state: &SettingsState, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = self.prefs.clone();
        v_flex()
            .gap(px(22.))
            .child(section("Layout", None))
            .child(setting_row(
                "Focused Inbox",
                "Sort the Inbox into Focused and Other.",
                Switch::new("pref-focused").checked(self.focused_enabled).on_click(cx.listener(|this, _checked: &bool, _w, cx| this.toggle_focused_inbox(cx))).into_any_element(),
            ))
            .child(setting_row(
                "Sender pictures",
                "Show initials next to each conversation.",
                Switch::new("pref-avatars").checked(prefs.show_avatars).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.show_avatars = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(setting_row(
                "Message preview text",
                "Show the first line of each conversation in the list.",
                Switch::new("pref-previews").checked(prefs.show_previews).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.show_previews = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(section("Reading", None))
            .child(setting_row(
                "Mark as read",
                "When a conversation is opened in the reading pane.",
                choice(&[("When selected", MarkAsRead::WhenSelected), ("After 3 seconds", MarkAsRead::AfterDelay), ("Never", MarkAsRead::Never)], prefs.mark_as_read, cx, |this, v, _w, cx| {
                    this.prefs.mark_as_read = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Remote images",
                "Images hosted on the sender’s servers can reveal when you open a message.",
                choice(&[("Block until I choose", RemoteImages::Block), ("Always show", RemoteImages::Allow)], prefs.remote_images, cx, |this, v, _w, cx| {
                    this.prefs.remote_images = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(section("Compose and reply", None))
            .child(setting_row(
                "Default reply",
                "What the reply box and Ctrl R do.",
                choice(&[("Reply", false), ("Reply all", true)], prefs.reply_all_by_default, cx, |this, v, _w, cx| {
                    this.prefs.reply_all_by_default = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Quote the original message in replies",
                "",
                Switch::new("pref-quote").checked(prefs.quote_on_reply).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.quote_on_reply = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(section("Attachments", None))
            .child(setting_row("Save attachments to", "Folder offered first when you save an attachment. Leave empty for Downloads.", Input::new(&state.attachments_dir).w(px(320.)).into_any_element()))
            .child(section("Download", None))
            .child(setting_row(
                "Inbox messages to download",
                "Newest messages fetched for each account; Sent, Archive, and other mailboxes scale with it. Larger numbers take longer on first sync.",
                choice(&[("100", 100u32), ("300", 300), ("1,000", 1000)], prefs.inbox_messages, cx, |this, v, window, cx| {
                    this.prefs.inbox_messages = v;
                    this.save_prefs(cx);
                    this.rebuild_providers_for_download_size(window, cx);
                }),
            ))
    }

    fn render_settings_calendar(&self, state: &SettingsState, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = self.prefs.clone();
        v_flex()
            .gap(px(22.))
            .child(section("Working hours", Some("Hours outside this range are shaded in Day and Week views.")))
            .child(setting_row("Start", "", Input::new(&state.work_start).w(px(140.)).into_any_element()))
            .child(setting_row("End", "", Input::new(&state.work_end).w(px(140.)).into_any_element()))
            .child(section("Events", None))
            .child(setting_row(
                "Default event length",
                "",
                choice(&[("15 min", 15u32), ("30 min", 30), ("45 min", 45), ("1 hour", 60)], prefs.default_event_minutes, cx, |this, v, _w, cx| {
                    this.prefs.default_event_minutes = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Default reminder",
                "Used when an event or calendar sets no reminder of its own.",
                choice(&[("None", 0u32), ("5 min", 5), ("10 min", 10), ("15 min", 15), ("30 min", 30)], prefs.default_reminder_minutes, cx, |this, v, _w, cx| {
                    this.prefs.default_reminder_minutes = v;
                    this.save_prefs(cx);
                }),
            ))
            .child(setting_row(
                "Show declined events",
                "",
                Switch::new("pref-declined").checked(prefs.show_declined_events).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.show_declined_events = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(setting_row(
                "Event reminders",
                "Desktop notification before events start.",
                Switch::new("pref-reminders-cal").checked(prefs.event_reminders).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.event_reminders = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
    }

    fn render_settings_about(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let data_dir = crate::data_dir();
        let data_dir_text = data_dir.display().to_string();
        let config = crate::google_signin::client_config_path().display().to_string();
        v_flex()
            .gap(px(22.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(14.))
                    .child(super::brand_tile(44.))
                    .child(
                        v_flex()
                            .child(div().font_family("Source Serif 4 Display").text_size(px(26.)).child("Overlook"))
                            .child(div().text_size(px(13.)).text_color(p.muted).child("A clear view of what matters. Thoughtfully made for Linux."))
                            .child(div().text_size(px(12.)).text_color(p.muted).child(format!("Version {} · by Last Refuge", env!("CARGO_PKG_VERSION")))),
                    ),
            )
            .child(section("Updates", None))
            .child(setting_row(
                &format!("Overlook {} · {}", crate::updater::CURRENT_VERSION, self.update.install.describe()),
                self.update.last_result.as_deref().unwrap_or("Not checked yet."),
                h_flex()
                    .gap(px(6.))
                    .child(Button::new("check-updates").small().icon(icon("refresh-cw").size(px(14.))).label("Check for updates").loading(self.update.phase == super::UpdatePhase::Checking).on_click(cx.listener(|this, _, window, cx| this.check_for_updates(true, window, cx))))
                    .when(self.update.latest.is_some() && matches!(self.update.phase, super::UpdatePhase::Idle | super::UpdatePhase::Failed(_)), |r| {
                        r.child(Button::new("install-update").primary().small().label("Update now").on_click(cx.listener(|this, _, window, cx| {
                            this.close_settings(window, cx);
                            this.start_update(window, cx);
                        })))
                    })
                    .into_any_element(),
            ))
            .child(setting_row(
                "Check for updates automatically",
                "Once a day, from the project’s GitHub releases. Nothing about you is sent.",
                Switch::new("pref-updates").checked(self.prefs.check_for_updates).on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.prefs.check_for_updates = *checked;
                    this.save_prefs(cx);
                })).into_any_element(),
            ))
            .child(section("Links", None))
            .child(setting_row("Last Refuge", "", Button::new("about-site").ghost().small().icon(icon("external-link").size(px(14.))).label("lastrefuge.ai").on_click(|_, _, cx| cx.open_url("https://lastrefuge.ai/")).into_any_element()))
            .child(setting_row("Source", "", Button::new("about-repo").ghost().small().icon(icon("external-link").size(px(14.))).label("github.com/last-refuge/overlook").on_click(|_, _, cx| cx.open_url("https://github.com/last-refuge/overlook")).into_any_element()))
            .child(section("On this device", None))
            .child(setting_row("Mail, events, drafts, and attachments", &data_dir_text, Button::new("about-open-data").ghost().small().icon(icon("folder-open").size(px(14.))).label("Open folder").on_click(move |_, _, cx| cx.reveal_path(&data_dir)).into_any_element()))
            .child(setting_row("Google OAuth client", &config, div().into_any_element()))
            .child(setting_row("Sign-in tokens", "Stored in the system keyring (Secret Service), never in files.", div().into_any_element()))
            .child(section("Keyboard", None))
            .child(div().text_size(px(12.)).text_color(p.muted).line_height(px(20.)).child(
                "Ctrl N new message · Ctrl E search · Ctrl R reply · Ctrl Shift R reply all · Ctrl F forward · Ctrl Enter send · Ctrl U toggle read · J/K move · E archive or move to Inbox · Delete · S star · Ctrl 1 Mail · Ctrl 2 Calendar · Ctrl Shift A new event · Ctrl , settings · in the calendar: ← → T D W M A",
            ))
    }
}

fn hour_text(hour: u8) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h12}:00 {suffix}")
}

fn section(title: &str, description: Option<&str>) -> Div {
    v_flex()
        .gap(px(2.))
        .pt(px(6.))
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title.to_owned()))
        .children(description.map(|d| div().text_size(px(12.)).child(d.to_owned()).opacity(0.7)))
}

fn setting_row(label: &str, description: &str, control: AnyElement) -> Div {
    h_flex()
        .items_center()
        .gap(px(24.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(label.to_owned()))
                .when(!description.is_empty(), |d| d.child(div().text_size(px(12.)).opacity(0.7).child(description.to_owned()))),
        )
        .child(div().flex_none().child(control))
}

/// Segmented choice control.
fn choice<T: Copy + PartialEq + 'static>(
    options: &[(&'static str, T)],
    current: T,
    cx: &mut Context<MailWindow>,
    on_select: impl Fn(&mut MailWindow, T, &mut Window, &mut Context<MailWindow>) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    h_flex()
        .gap(px(2.))
        .p(px(3.))
        .rounded(px(8.))
        .bg(cx.palette().sidebar)
        .border_1()
        .border_color(cx.palette().divider)
        .children(options.iter().map(|(label, value)| {
            let value = *value;
            let on_select = on_select.clone();
            Button::new(SharedString::from(format!("choice-{label}")))
                .ghost()
                .small()
                .label(*label)
                .toggled(value == current)
                .on_click(cx.listener(move |this, _, window, cx| on_select(this, value, window, cx)))
        }))
        .into_any_element()
}
