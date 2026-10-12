//! Calendar module: week, day, and agenda views, event details with responses, and a
//! simple new-event form. Everything reads from the local store; changes are written with
//! their pending operation and reach Google through the sync engine.

use super::widgets::*;
use super::MailWindow;
use crate::assets::icon;
use crate::theme::ActivePalette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::*;
use overlook_core::sanitize::{sanitize_html, text_to_html, RemoteContent};
use overlook_core::time::now;
use overlook_core::tz;
use overlook_core::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Module {
    Mail,
    Calendar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalView {
    Day,
    Week,
    Month,
    Agenda,
}

const HOUR_PX: f32 = 52.;
const GUTTER_PX: f32 = 56.;
const AGENDA_DAYS: i32 = 14;

pub struct NewEvent {
    pub title: Entity<InputState>,
    pub date: Entity<InputState>,
    pub start: Entity<InputState>,
    pub end: Entity<InputState>,
    pub location: Entity<InputState>,
    pub description: Entity<TextareaState>,
    pub calendar: Option<CalendarId>,
    pub all_day: bool,
    pub error: Option<String>,
    /// `Some` when editing an existing event.
    pub editing: Option<EventId>,
}

fn clock_text(ts: i64) -> String {
    tz::format_clock(ts)
}

fn hex_to_hsla(hex: &str) -> Option<Hsla> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    Some(Rgba::from(gpui_kit::rgb(v)).into())
}

impl MailWindow {
    // ------------------------------------------------------------------ state

    pub(crate) fn cal_range(&self) -> (i64, i64) {
        match self.cal_view {
            CalView::Day => (self.cal_anchor, tz::add_days(self.cal_anchor, 1)),
            CalView::Week => {
                let start = tz::week_start(self.cal_anchor, self.prefs.week_starts_monday);
                (start, tz::add_days(start, 7))
            }
            CalView::Month => {
                let start = tz::week_start(tz::month_start(self.cal_anchor), self.prefs.week_starts_monday);
                (start, tz::add_days(start, 42))
            }
            CalView::Agenda => (self.cal_anchor, tz::add_days(self.cal_anchor, AGENDA_DAYS)),
        }
    }

    pub(crate) fn reload_calendar(&mut self) {
        self.calendars = self.store.calendars(&self.account.id).unwrap_or_default();
        let (from, to) = self.cal_range();
        self.cal_events = self.store.events_between(&self.account.id, from, to).unwrap_or_default();
        if !self.prefs.show_declined_events {
            self.cal_events.retain(|e| e.my_response() != Some(ResponseStatus::Declined));
        }
        self.cal_status = crate::sync::calendar_status(&self.store, &self.account.id);
        if let Some(sel) = &self.cal_selected {
            if self.store.event(sel).ok().flatten().is_none() {
                self.cal_selected = None;
            }
        }
    }

    pub(crate) fn set_module(&mut self, module: Module, window: &mut Window, cx: &mut Context<Self>) {
        if self.module == module {
            return;
        }
        self.module = module;
        let _ = self.store.set_setting("last_module", &if module == Module::Calendar { "calendar" } else { "mail" });
        match module {
            Module::Calendar => {
                self.reload_calendar();
                self.cal_focus.focus(window, cx);
            }
            Module::Mail => self.list_focus.focus(window, cx),
        }
        cx.notify();
    }

    pub(crate) fn set_cal_view(&mut self, view: CalView, cx: &mut Context<Self>) {
        self.cal_view = view;
        self.reload_calendar();
        cx.notify();
    }

    pub(crate) fn cal_step(&mut self, direction: i32, cx: &mut Context<Self>) {
        self.cal_anchor = match self.cal_view {
            CalView::Day => tz::add_days(self.cal_anchor, direction),
            CalView::Week => tz::add_days(self.cal_anchor, direction * 7),
            CalView::Month => tz::add_months(tz::month_start(self.cal_anchor), direction),
            CalView::Agenda => tz::add_days(self.cal_anchor, direction * AGENDA_DAYS),
        };
        self.reload_calendar();
        cx.notify();
    }

    pub(crate) fn cal_today(&mut self, cx: &mut Context<Self>) {
        self.cal_anchor = tz::day_start(now());
        self.reload_calendar();
        cx.notify();
    }

    pub(crate) fn toggle_calendar_visible(&mut self, id: CalendarId, cx: &mut Context<Self>) {
        if let Some(c) = self.calendars.iter().find(|c| c.id == id) {
            let _ = self.store.set_calendar_visible(&id, !c.visible);
        }
        self.reload_calendar();
        cx.notify();
    }

    fn selected_event(&self) -> Option<&Event> {
        let id = self.cal_selected.as_ref()?;
        self.cal_events.iter().find(|e| &e.id == id)
    }

    fn calendar_of(&self, event: &Event) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == event.calendar_id)
    }

    /// Switches to the calendar, moves to the event's day, and selects it.
    pub(crate) fn show_event_in_calendar(&mut self, id: EventId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self.store.event(&id).ok().flatten() else { return };
        self.set_module(Module::Calendar, window, cx);
        self.cal_anchor = tz::day_start(event.start);
        if self.cal_view == CalView::Agenda {
            self.cal_view = CalView::Week;
        }
        self.new_event = None;
        self.reload_calendar();
        self.cal_selected = Some(id);
        cx.notify();
    }

    /// Fires due reminders: a desktop notification plus an in-app toast, once per event
    /// and offset per session.
    pub(crate) fn check_reminders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reminders_enabled {
            return;
        }
        let now_ms = now();
        let horizon = now_ms + 2 * 3_600_000;
        let accounts = self.accounts.clone();
        for account in accounts {
            let calendars = self.store.calendars(&account.id).unwrap_or_default();
            let Ok(events) = self.store.events_starting_between(&account.id, now_ms - 60_000, horizon) else { continue };
            for event in events {
                if event.my_response() == Some(ResponseStatus::Declined) || event.is_working_location() {
                    continue;
                }
                // Google reports no default for some calendars; 10 minutes is Google's own
                // default and close to Outlook's 15, and reminders can be switched off.
                let fallback = self.prefs.default_reminder_minutes as i32;
                let defaults: Vec<i32> = calendars.iter().find(|c| c.id == event.calendar_id).map(|c| c.default_reminders.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| if fallback > 0 { vec![fallback] } else { Vec::new() });
                let offsets: Vec<i32> = event.effective_reminders(&defaults).to_vec();
                for minutes in offsets {
                    let fire_at = event.start - i64::from(minutes) * 60_000;
                    let key = (event.id.clone(), minutes);
                    if fire_at <= now_ms && event.start > now_ms - 60_000 && !self.notified.contains(&key) {
                        self.notified.insert(key);
                        let remaining = ((event.start - now_ms) / 60_000).max(0);
                        let when = if remaining == 0 { "now".to_owned() } else { format!("in {remaining} min") };
                        let title = event.display_title();
                        let mut body = format!("{} · {} – {}", when, tz::format_clock(event.start), tz::format_clock(event.end));
                        if let Some(l) = &event.location {
                            body.push_str(&format!("\n{l}"));
                        }
                        crate::notify::desktop(title.clone(), body);
                        window.push_notification(Notification::info(format!("{title} · {when}")), cx);
                    }
                }
            }
        }
    }

    pub(crate) fn start_edit_event(&mut self, id: EventId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self.store.event(&id).ok().flatten() else { return };
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Add a title").default_value(event.summary.clone()));
        let date = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(tz::local_date(event.start).to_string()));
        let start = cx.new(|cx| InputState::new(window, cx).placeholder("9:00 AM").default_value(clock_text(event.start)));
        let end = cx.new(|cx| InputState::new(window, cx).placeholder("10:00 AM").default_value(clock_text(if event.all_day { event.start + 3_600_000 } else { event.end })));
        let location = cx.new(|cx| InputState::new(window, cx).placeholder("Add a location").default_value(event.location.clone().unwrap_or_default()));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Add a description").default_value(event.description.clone().unwrap_or_default()));
        title.update(cx, |s, cx| s.focus(window, cx));
        self.new_event = Some(NewEvent { title, date, start, end, location, description, calendar: Some(event.calendar_id.clone()), all_day: event.all_day, error: None, editing: Some(id) });
        cx.notify();
    }

    pub(crate) fn respond_to_event(&mut self, id: EventId, response: ResponseStatus, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut event) = self.store.event(&id).ok().flatten() else { return };
        let mut found = false;
        for a in &mut event.attendees {
            if a.is_self {
                a.response = response;
                found = true;
            }
        }
        if !found {
            event.attendees.push(Attendee { email: self.account.email.clone(), name: Some(self.account.display_name.clone()), response, is_self: true, is_organizer: false, optional: false });
        }
        match self.store.apply_event_change(&self.account.id, &event, OperationKind::RespondToEvent { event_id: id, response }) {
            Ok(_) => {
                self.reload_calendar();
                cx.notify();
                if self.connected(&self.account.id) {
                    self.sync_now(window, cx);
                }
            }
            Err(e) => window.push_notification(Notification::error(format!("Couldn’t save your response: {e}")), cx),
        }
    }

    pub(crate) fn delete_event(&mut self, id: EventId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self.store.event(&id).ok().flatten() else { return };
        match self.store.delete_event_locally(&self.account.id, &event) {
            Ok(_) => {
                self.cal_selected = None;
                self.reload_calendar();
                cx.notify();
                window.push_notification(Notification::info(format!("Deleted “{}”.", event.display_title())), cx);
                if self.connected(&self.account.id) {
                    self.sync_now(window, cx);
                }
            }
            Err(e) => window.push_notification(Notification::error(format!("Couldn’t delete the event: {e}")), cx),
        }
    }

    pub(crate) fn start_new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editable: Vec<&Calendar> = self.calendars.iter().filter(|c| c.can_edit).collect();
        if editable.is_empty() {
            window.push_notification(Notification::warning("No calendar you can write to is connected yet."), cx);
            return;
        }
        let calendar = editable.iter().find(|c| c.is_primary).or(editable.first()).map(|c| c.id.clone());
        let day = if self.cal_view == CalView::Week { tz::day_start(now()).max(self.cal_range().0).min(tz::add_days(self.cal_range().1, -1)) } else { self.cal_anchor };
        let date_text = tz::local_date(day).to_string();
        let next_hour = {
            let z = tz::zoned(now());
            let h = (z.hour() as i32 + 1).min(23);
            format!("{}:00 {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
        };
        let end_hour = {
            let z = tz::zoned(now());
            let start_minutes = ((z.hour() as i32 + 1).min(23)) * 60;
            let end_minutes = (start_minutes + self.prefs.default_event_minutes as i32).min(23 * 60 + 59);
            let (h, m) = (end_minutes / 60, end_minutes % 60);
            format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, m, if h < 12 { "AM" } else { "PM" })
        };
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Add a title"));
        let date = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD").default_value(date_text));
        let start = cx.new(|cx| InputState::new(window, cx).placeholder("9:00 AM").default_value(next_hour));
        let end = cx.new(|cx| InputState::new(window, cx).placeholder("10:00 AM").default_value(end_hour));
        let location = cx.new(|cx| InputState::new(window, cx).placeholder("Add a location"));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Add a description"));
        title.update(cx, |s, cx| s.focus(window, cx));
        self.cal_selected = None;
        self.new_event = Some(NewEvent { title, date, start, end, location, description, calendar, all_day: false, error: None, editing: None });
        cx.notify();
    }

    pub(crate) fn cancel_new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_event = None;
        self.cal_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn save_new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.new_event.as_ref() else { return };
        let title = form.title.read(cx).value().trim().to_owned();
        let date_text = form.date.read(cx).value().trim().to_owned();
        let start_text = form.start.read(cx).value().trim().to_owned();
        let end_text = form.end.read(cx).value().trim().to_owned();
        let location = form.location.read(cx).value().trim().to_owned();
        let description = form.description.read(cx).value().trim().to_owned();
        let all_day = form.all_day;
        let editing = form.editing.clone();
        let calendar = form.calendar.clone().and_then(|id| self.calendars.iter().find(|c| c.id == id).cloned());
        let fail = |msg: &str, this: &mut Self, cx: &mut Context<Self>| {
            if let Some(f) = this.new_event.as_mut() {
                f.error = Some(msg.to_owned());
            }
            cx.notify();
        };
        let Some(calendar) = calendar else { return fail("Choose a calendar.", self, cx) };
        let Some(date) = tz::parse_date(&date_text) else { return fail("Enter the date as YYYY-MM-DD.", self, cx) };
        let (start, end) = if all_day {
            let s = tz::date_start(date);
            (s, tz::add_days(s, 1))
        } else {
            let Some(st) = tz::parse_clock(&start_text) else { return fail("Enter a start time like 9:30 AM.", self, cx) };
            let Some(et) = tz::parse_clock(&end_text) else { return fail("Enter an end time like 10:00 AM.", self, cx) };
            let s = tz::local_datetime(date, st);
            let e = tz::local_datetime(date, et);
            if e <= s {
                return fail("The end time must be after the start time.", self, cx);
            }
            (s, e)
        };
        if let Some(existing_id) = editing {
            let Some(mut existing) = self.store.event(&existing_id).ok().flatten() else { return fail("This event no longer exists.", self, cx) };
            existing.summary = if title.is_empty() { "(No title)".into() } else { title };
            existing.location = if location.is_empty() { None } else { Some(location) };
            existing.description = if description.is_empty() { None } else { Some(description) };
            existing.start = start;
            existing.end = end;
            existing.all_day = all_day;
            existing.updated = now();
            match self.store.apply_event_change(&self.account.id, &existing, OperationKind::UpdateEvent { event_id: existing_id.clone() }) {
                Ok(_) => {
                    self.new_event = None;
                    self.cal_anchor = tz::day_start(start);
                    self.reload_calendar();
                    self.cal_selected = Some(existing_id);
                    self.cal_focus.focus(window, cx);
                    cx.notify();
                    window.push_notification(Notification::info("Saved on this device. Syncing to your calendar…"), cx);
                    if self.connected(&self.account.id) {
                        self.sync_now(window, cx);
                    }
                }
                Err(e) => fail(&format!("Couldn’t save the event: {e}"), self, cx),
            }
            return;
        }
        let pid = new_provider_event_id();
        let event = Event {
            id: EventId(format!("{}:{pid}", calendar.id.0)),
            account_id: self.account.id.clone(),
            calendar_id: calendar.id.clone(),
            provider_id: Some(ProviderId(pid)),
            ical_uid: None,
            summary: if title.is_empty() { "(No title)".into() } else { title },
            description: if description.is_empty() { None } else { Some(description) },
            location: if location.is_empty() { None } else { Some(location) },
            start,
            end,
            all_day,
            status: EventStatus::Confirmed,
            organizer: Some(self.owner()),
            attendees: Vec::new(),
            html_link: None,
            conference_url: None,
            recurring_event_id: None,
            updated: now(),
            event_type: None,
            color: None,
            reminders: None,
            pending: true,
        };
        match self.store.apply_event_change(&self.account.id, &event, OperationKind::CreateEvent { event_id: event.id.clone() }) {
            Ok(_) => {
                self.new_event = None;
                self.cal_anchor = tz::day_start(start);
                self.reload_calendar();
                self.cal_selected = Some(event.id.clone());
                self.cal_focus.focus(window, cx);
                cx.notify();
                window.push_notification(Notification::info("Saved on this device. Syncing to your calendar…"), cx);
                if self.connected(&self.account.id) {
                    self.sync_now(window, cx);
                }
            }
            Err(e) => fail(&format!("Couldn’t save the event: {e}"), self, cx),
        }
    }

    // --------------------------------------------------------------- sidebar

    pub(crate) fn render_calendar_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (dot, text) = match &self.cal_status {
            SyncState::UpToDate => (p.success, "Up to date".to_owned()),
            SyncState::Syncing => (p.accent, "Syncing…".to_owned()),
            SyncState::Paused(_) => (p.accent, "Paused".to_owned()),
            SyncState::Offline => (p.muted, "Offline".to_owned()),
            SyncState::SignInRequired => (p.warning, "Sign-in required".to_owned()),
            SyncState::SyncFailed(r) => (p.warning, format!("Sync failed: {r}")),
        };
        let mut list = v_flex().gap(px(2.));
        for c in &self.calendars {
            let id = c.id.clone();
            let color = c.color.as_deref().and_then(hex_to_hsla).unwrap_or(p.accent);
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("cal-{}", c.id)))
                    .items_center()
                    .gap(px(10.))
                    .min_h(px(32.))
                    .px(px(11.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(if c.visible { p.foreground } else { p.muted })
                    .hover(|d| d.bg(p.hover))
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.))
                            .rounded(px(4.))
                            .border_2()
                            .border_color(color)
                            .when(c.visible, |d| d.bg(color))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(c.visible, |d| d.child(icon("check").size(px(10.)).text_color(gpui_kit::white()))),
                    )
                    .child(div().flex_1().truncate().child(c.name.clone()))
                    .on_click(cx.listener(move |this, _, _w, cx| this.toggle_calendar_visible(id.clone(), cx))),
            );
        }
        let owner = self.owner();
        v_flex()
            .size_full()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.divider)
            .pt(px(18.))
            .px(px(11.))
            .pb(px(14.))
            .child(
                Button::new("new-event")
                    .primary()
                    .icon(icon("calendar-plus").size(px(16.)))
                    .label("New event")
                    .tooltip("New event (Ctrl Shift A)")
                    .w_full()
                    .h(px(38.))
                    .on_click(cx.listener(|this, _, window, cx| this.start_new_event(window, cx))),
            )
            .child(div().mt(px(18.)).mb(px(6.)).px(px(11.)).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child("My calendars"))
            .child(v_flex().id("calendar-list").flex_1().min_h_0().child(list).overflow_y_scrollbar())
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
                                    .child(div().size(px(6.)).rounded_full().bg(dot))
                                    .child(format!("Calendar · {text}")),
                            ),
                    ),
            )
    }

    // ------------------------------------------------------------------ main

    pub(crate) fn render_calendar_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (from, to) = self.cal_range();
        let title = match self.cal_view {
            CalView::Day => tz::zoned(self.cal_anchor).strftime("%A, %B %-d, %Y").to_string(),
            CalView::Week => tz::format_week_range(from),
            CalView::Month => tz::format_month_year(self.cal_anchor),
            CalView::Agenda => format!("{} – {}", tz::format_day(from), tz::format_day(tz::add_days(to, -1))),
        };
        let view_button = |label: &'static str, view: CalView, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("view-{label}")))
                .ghost()
                .small()
                .label(label)
                .toggled(self.cal_view == view)
                .on_click(cx.listener(move |this, _, _w, cx| this.set_cal_view(view, cx)))
        };
        let toolbar = h_flex()
            .items_center()
            .gap(px(6.))
            .h(px(52.))
            .px(px(16.))
            .border_b_1()
            .border_color(p.divider)
            .child(Button::new("cal-today").small().label("Today").on_click(cx.listener(|this, _, _w, cx| this.cal_today(cx))))
            .child(Button::new("cal-prev").ghost().small().icon(icon("chevron-left").size(px(16.))).tooltip("Previous (←)").on_click(cx.listener(|this, _, _w, cx| this.cal_step(-1, cx))))
            .child(Button::new("cal-next").ghost().small().icon(icon("chevron-right").size(px(16.))).tooltip("Next (→)").on_click(cx.listener(|this, _, _w, cx| this.cal_step(1, cx))))
            .child(div().ml(px(8.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(px(2.))
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(p.sidebar)
                    .border_1()
                    .border_color(p.divider)
                    .child(view_button("Day", CalView::Day, cx))
                    .child(view_button("Week", CalView::Week, cx))
                    .child(view_button("Month", CalView::Month, cx))
                    .child(view_button("Agenda", CalView::Agenda, cx)),
            );

        let banner = match &self.cal_status {
            SyncState::SignInRequired if self.account.provider == ProviderKind::Gmail => {
                let email = self.account.email.clone();
                Some(
                    h_flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(16.))
                        .h(px(40.))
                        .bg(p.chip)
                        .border_b_1()
                        .border_color(p.divider)
                        .text_size(px(13.))
                        .child(icon("key-round").size(px(16.)).text_color(p.warning))
                        .child(div().flex_1().child(crate::sync::CALENDAR_SIGNIN_WORDING))
                        .child(Button::new("cal-signin").small().label("Sign in again").on_click(cx.listener(move |this, _, window, cx| this.start_google_sign_in(Some(email.clone()), window, cx)))),
                )
            }
            _ if self.account.provider == ProviderKind::Sample => Some(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .h(px(40.))
                    .bg(p.chip)
                    .border_b_1()
                    .border_color(p.divider)
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child(icon("info").size(px(16.)).text_color(p.muted))
                    .child("The sample account has no calendar. Switch to a Gmail account to see your events."),
            ),
            _ => None,
        };

        let grid: AnyElement = match self.cal_view {
            CalView::Day => self.render_time_grid(from, 1, cx).into_any_element(),
            CalView::Week => self.render_time_grid(from, 7, cx).into_any_element(),
            CalView::Month => self.render_month_grid(from, cx).into_any_element(),
            CalView::Agenda => self.render_agenda(from, cx).into_any_element(),
        };

        let detail: Option<AnyElement> = if self.new_event.is_some() {
            Some(self.render_new_event(cx).into_any_element())
        } else {
            self.selected_event().map(|e| self.render_event_detail(e, cx).into_any_element())
        };

        v_flex()
            .id("calendar-main")
            .key_context(super::CALENDAR_CONTEXT)
            .track_focus(&self.cal_focus)
            .size_full()
            .bg(p.surface)
            .child(toolbar)
            .children(banner)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(grid))
                    .children(detail.map(|d| div().w(px(360.)).flex_none().h_full().border_l_1().border_color(p.divider).child(d))),
            )
    }

    fn render_time_grid(&self, from: i64, days: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let today = tz::day_start(now());
        let day_starts: Vec<i64> = (0..days as i32).map(|d| tz::add_days(from, d)).collect();

        // Scroll to the working hours the first time a view/period is shown.
        let key = (self.cal_view, from);
        if self.cal_scrolled_for.get() != Some(key) {
            self.cal_scroll.set_offset(point(px(0.), px(-7.5 * HOUR_PX)));
            self.cal_scrolled_for.set(Some(key));
        }

        let header = h_flex()
            .pl(px(GUTTER_PX))
            .border_b_1()
            .border_color(p.divider)
            .children(day_starts.iter().map(|&d| {
                let z = tz::zoned(d);
                let is_today = d == today;
                let next = tz::add_days(d, 1);
                let location: Option<String> = self.cal_events.iter().find(|e| e.is_working_location() && e.overlaps(d, next)).map(|e| e.display_title());
                v_flex()
                    .flex_1()
                    .items_center()
                    .py(px(8.))
                    .gap(px(2.))
                    .child(div().text_size(px(11.)).text_color(if is_today { p.accent } else { p.muted }).child(z.strftime("%a").to_string().to_uppercase()))
                    .child(
                        div()
                            .size(px(28.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(is_today, |d| d.bg(p.action).text_color(p.on_action))
                            .child(z.day().to_string()),
                    )
                    .children(location.map(|l| {
                        h_flex()
                            .items_center()
                            .gap(px(3.))
                            .text_size(px(10.))
                            .text_color(p.muted)
                            .child(icon("map-pin").size(px(10.)).text_color(p.muted))
                            .child(l)
                    }))
            }));

        // All-day and multi-day events.
        let all_day: Vec<&Event> = self.cal_events.iter().filter(|e| !e.is_working_location() && (e.all_day || e.end - e.start >= 86_400_000)).collect();
        let all_day_row = (!all_day.is_empty()).then(|| {
            h_flex()
                .pl(px(GUTTER_PX))
                .min_h(px(28.))
                .border_b_1()
                .border_color(p.divider)
                .children(day_starts.iter().map(|&d| {
                    let next = tz::add_days(d, 1);
                    let here: Vec<&Event> = all_day.iter().copied().filter(|e| e.overlaps(d, next)).collect();
                    v_flex().flex_1().min_w_0().gap(px(2.)).p(px(2.)).children(here.into_iter().map(|e| self.event_chip(e, cx)))
                }))
        });

        let hours = v_flex().w(px(GUTTER_PX)).flex_none().relative().h(px(24. * HOUR_PX)).children((1..24).map(|h| {
            div()
                .absolute()
                .top(px(h as f32 * HOUR_PX - 7.))
                .right(px(8.))
                .text_size(px(10.))
                .text_color(p.muted)
                .child(format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }))
        }));

        let now_ms = now();
        let columns = day_starts.iter().map(|&d| {
            let next = tz::add_days(d, 1);
            let timed: Vec<&Event> = self.cal_events.iter().filter(|e| !e.all_day && e.end - e.start < 86_400_000 && e.overlaps(d, next)).collect();
            let lanes = assign_lanes(&timed);
            let mut col = div().flex_1().min_w_0().relative().h(px(24. * HOUR_PX)).border_l_1().border_color(p.divider);
            let (ws, we) = (self.prefs.work_start_hour.min(24) as f32, self.prefs.work_end_hour.min(24) as f32);
            if ws > 0. {
                col = col.child(div().absolute().top_0().left_0().right_0().h(px(ws * HOUR_PX)).bg(p.background));
            }
            if we < 24. {
                col = col.child(div().absolute().top(px(we * HOUR_PX)).left_0().right_0().h(px((24. - we) * HOUR_PX)).bg(p.background));
            }
            for h in 0..24 {
                col = col.child(div().absolute().top(px(h as f32 * HOUR_PX)).left_0().right_0().h(px(1.)).bg(p.divider));
            }
            for (e, (lane, lanes_total)) in timed.iter().zip(lanes) {
                let start_min = if e.start < d { 0 } else { tz::minutes_into_day(e.start) } as f32;
                let end_min = if e.end > next { 24. * 60. } else { tz::minutes_into_day(e.end) as f32 }.max(start_min + 20.);
                let top = start_min / 60. * HOUR_PX;
                let height = (end_min - start_min) / 60. * HOUR_PX;
                let width = 1.0 / lanes_total as f32;
                col = col.child(
                    div()
                        .absolute()
                        .top(px(top))
                        .h(px(height - 2.))
                        .left(relative(lane as f32 * width))
                        .w(relative(width))
                        .pr(px(4.))
                        .child(self.event_block(e, height, cx)),
                );
            }
            if d == today {
                let m = tz::minutes_into_day(now_ms) as f32;
                col = col.child(div().absolute().top(px(m / 60. * HOUR_PX - 1.)).left_0().right_0().h(px(2.)).bg(p.brand_accent));
                col = col.child(div().absolute().top(px(m / 60. * HOUR_PX - 4.)).left(px(-4.)).size(px(8.)).rounded_full().bg(p.brand_accent));
            }
            col
        });

        v_flex()
            .size_full()
            .child(header)
            .children(all_day_row)
            .child(
                div()
                    .id("cal-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.cal_scroll)
                    .child(h_flex().items_start().child(hours).children(columns)),
            )
    }

    fn event_colors(&self, e: &Event, p: &crate::theme::Palette) -> (Hsla, Hsla) {
        let base = e
            .color
            .as_deref()
            .and_then(hex_to_hsla)
            .or_else(|| self.calendar_of(e).and_then(|c| c.color.as_deref()).and_then(hex_to_hsla))
            .unwrap_or(p.accent);
        let mut fill = base;
        fill.a = if p.is_dark() { 0.28 } else { 0.18 };
        let mut edge = base;
        if p.is_dark() {
            edge.l = edge.l.max(0.6);
        } else {
            edge.l = edge.l.min(0.42);
        }
        (fill, edge)
    }

    fn event_block(&self, e: &Event, height: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (fill, edge) = self.event_colors(e, &p);
        let selected = self.cal_selected.as_ref() == Some(&e.id);
        let declined = e.my_response() == Some(ResponseStatus::Declined);
        let tentative = matches!(e.my_response(), Some(ResponseStatus::NeedsAction) | Some(ResponseStatus::Tentative)) && e.is_invitation();
        let id = e.id.clone();
        div()
            .id(SharedString::from(format!("ev-{}", e.id)))
            .size_full()
            .rounded(px(6.))
            .bg(fill)
            .border_l_2()
            .border_color(edge)
            .when(selected, |d| d.border_1().border_color(edge).bg(fill.opacity(1.0)))
            .when(tentative, |d| d.border_dashed())
            .when(declined, |d| d.opacity(0.5))
            .px(px(6.))
            .py(px(3.))
            .overflow_hidden()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(p.foreground)
            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(e.display_title()))
            .when(height >= 36., |d| d.child(div().text_size(px(11.)).text_color(p.muted).truncate().child(format!("{} – {}", tz::format_clock(e.start), tz::format_clock(e.end)))))
            .when(height >= 54. && e.location.is_some(), |d| d.child(div().text_size(px(11.)).text_color(p.muted).truncate().child(e.location.clone().unwrap_or_default())))
            .on_click(cx.listener(move |this, _, _w, cx| {
                this.new_event = None;
                this.cal_selected = Some(id.clone());
                cx.notify();
            }))
    }

    fn event_chip(&self, e: &Event, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (fill, edge) = self.event_colors(e, &p);
        let id = e.id.clone();
        div()
            .id(SharedString::from(format!("chip-{}", e.id)))
            .px(px(6.))
            .py(px(2.))
            .rounded(px(5.))
            .bg(fill)
            .border_l_2()
            .border_color(edge)
            .text_size(px(11.))
            .truncate()
            .cursor_pointer()
            .child(e.display_title())
            .on_click(cx.listener(move |this, _, _w, cx| {
                this.new_event = None;
                this.cal_selected = Some(id.clone());
                cx.notify();
            }))
    }

    fn render_month_grid(&self, from: i64, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let today = tz::day_start(now());
        let month = tz::local_date(self.cal_anchor).month();
        let names: [&str; 7] = if self.prefs.week_starts_monday { ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] } else { ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] };
        let header = h_flex().border_b_1().border_color(p.divider).children(names.iter().map(|d| {
            div().flex_1().py(px(6.)).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(div().px(px(8.)).child(d.to_uppercase()))
        }));
        let mut rows = v_flex().flex_1().min_h_0();
        for week in 0..6 {
            let mut row = h_flex().flex_1().min_h_0().border_b_1().border_color(p.divider);
            for dow in 0..7 {
                let day = tz::add_days(from, week * 7 + dow);
                let next = tz::add_days(day, 1);
                let z = tz::zoned(day);
                let in_month = z.month() == month;
                let is_today = day == today;
                let mut events: Vec<&Event> = self.cal_events.iter().filter(|e| !e.is_working_location() && e.overlaps(day, next)).collect();
                events.sort_by_key(|e| (!e.all_day, e.start));
                let shown = events.iter().take(3).copied().collect::<Vec<_>>();
                let more = events.len().saturating_sub(shown.len());
                let day_for_click = day;
                row = row.child(
                    v_flex()
                        .id(SharedString::from(format!("month-{day}")))
                        .flex_1()
                        .min_w_0()
                        .min_h(px(96.))
                        .p(px(4.))
                        .gap(px(2.))
                        .border_l_1()
                        .border_color(p.divider)
                        .when(!in_month, |d| d.bg(p.background))
                        .child(
                            h_flex().justify_end().child(
                                div()
                                    .id(SharedString::from(format!("month-day-{day}")))
                                    .size(px(24.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .text_color(if in_month { p.foreground } else { p.muted })
                                    .when(is_today, |d| d.bg(p.action).text_color(p.on_action).font_weight(FontWeight::SEMIBOLD))
                                    .hover(|d| d.bg(p.hover))
                                    .child(z.day().to_string())
                                    .on_click(cx.listener(move |this, _, _w, cx| {
                                        this.cal_anchor = day_for_click;
                                        this.set_cal_view(CalView::Day, cx);
                                    })),
                            ),
                        )
                        .children(shown.into_iter().map(|e| {
                            let (fill, edge) = self.event_colors(e, &p);
                            let id = e.id.clone();
                            let label = if e.all_day { e.display_title() } else { format!("{} {}", tz::format_clock(e.start), e.display_title()) };
                            div()
                                .id(SharedString::from(format!("mev-{}-{day}", e.id)))
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(fill)
                                .border_l_2()
                                .border_color(edge)
                                .text_size(px(11.))
                                .truncate()
                                .cursor_pointer()
                                .when(self.cal_selected.as_ref() == Some(&e.id), |d| d.bg(fill.opacity(1.0)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, _w, cx| {
                                    this.new_event = None;
                                    this.cal_selected = Some(id.clone());
                                    cx.notify();
                                }))
                        }))
                        .when(more > 0, |d| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!("more-{day}")))
                                    .px(px(5.))
                                    .text_size(px(11.))
                                    .text_color(p.muted)
                                    .cursor_pointer()
                                    .child(format!("+{more} more"))
                                    .on_click(cx.listener(move |this, _, _w, cx| {
                                        this.cal_anchor = day_for_click;
                                        this.set_cal_view(CalView::Day, cx);
                                    })),
                            )
                        }),
                );
            }
            rows = rows.child(row);
        }
        v_flex().id("month-grid").size_full().child(header).child(rows).overflow_y_scrollbar()
    }

    fn render_agenda(&self, from: i64, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let today = tz::day_start(now());
        let mut list = v_flex().id("agenda").px(px(20.)).py(px(12.)).gap(px(4.));
        let mut any = false;
        for d in 0..AGENDA_DAYS {
            let day = tz::add_days(from, d);
            let next = tz::add_days(day, 1);
            let events: Vec<&Event> = self.cal_events.iter().filter(|e| !e.is_working_location() && e.overlaps(day, next)).collect();
            if events.is_empty() {
                continue;
            }
            any = true;
            list = list.child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(14.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if day == today { p.accent } else { p.muted })
                    .child(tz::zoned(day).strftime("%A, %B %-d").to_string())
                    .when(day == today, |r| r.child(chip("Today", &p))),
            );
            for e in events {
                let (fill, edge) = self.event_colors(e, &p);
                let id = e.id.clone();
                let selected = self.cal_selected.as_ref() == Some(&e.id);
                let time = if e.all_day { "All day".to_owned() } else { format!("{} – {}", tz::format_clock(e.start), tz::format_clock(e.end)) };
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("ag-{}", e.id)))
                        .items_center()
                        .gap(px(12.))
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .when(selected, |r| r.bg(p.selection))
                        .when(!selected, |r| r.hover(|r| r.bg(p.hover)))
                        .child(div().w(px(4.)).h(px(32.)).rounded(px(2.)).bg(edge))
                        .child(div().w(px(130.)).text_size(px(12.)).text_color(p.muted).child(time))
                        .child(v_flex().flex_1().min_w_0().child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).truncate().child(e.display_title())).children(e.location.clone().map(|l| div().text_size(px(11.)).text_color(p.muted).truncate().child(l))))
                        .when(e.pending, |r| r.child(icon("cloud-off").size(px(13.)).text_color(p.warning)))
                        .child(div().size(px(10.)).rounded_full().bg(fill.opacity(1.0)))
                        .on_click(cx.listener(move |this, _, _w, cx| {
                            this.new_event = None;
                            this.cal_selected = Some(id.clone());
                            cx.notify();
                        })),
                );
            }
        }
        if !any {
            list = list.child(v_flex().items_center().pt(px(72.)).gap(px(6.)).text_color(p.muted).child(icon("calendar").size(px(28.)).text_color(p.divider)).child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.foreground).child("Nothing scheduled")).child(div().text_size(px(12.)).child("The next two weeks are clear.")));
        }
        list.overflow_y_scrollbar()
    }

    fn render_event_detail(&self, e: &Event, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (fill, edge) = self.event_colors(e, &p);
        let calendar = self.calendar_of(e);
        let when = if e.all_day {
            let days = ((e.end - e.start) / 86_400_000).max(1);
            if days == 1 { format!("{} · All day", tz::format_day(e.start)) } else { format!("{} – {} · All day", tz::format_day(e.start), tz::format_day(tz::add_days(e.end, -1))) }
        } else if tz::day_start(e.start) == tz::day_start(e.end) || e.end == tz::add_days(tz::day_start(e.start), 1) {
            format!("{} · {} – {}", tz::zoned(e.start).strftime("%A, %B %-d"), tz::format_clock(e.start), tz::format_clock(e.end))
        } else {
            format!("{} {} – {} {}", tz::format_day(e.start), tz::format_clock(e.start), tz::format_day(e.end), tz::format_clock(e.end))
        };
        let my = e.my_response();
        let id = e.id.clone();
        let can_edit = calendar.is_some_and(|c| e.editable_by(&self.account.email, c));
        let can_delete = can_edit;
        let fallback = self.prefs.default_reminder_minutes as i32;
        let defaults: Vec<i32> = calendar.map(|c| c.default_reminders.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| if fallback > 0 { vec![fallback] } else { Vec::new() });
        let reminders: Vec<i32> = e.effective_reminders(&defaults).to_vec();
        let reminder_text = if reminders.is_empty() || e.all_day {
            None
        } else {
            Some(reminders.iter().map(|m| match *m {
                0 => "at start".to_owned(),
                m if m % 1440 == 0 => format!("{} day{} before", m / 1440, if m == 1440 { "" } else { "s" }),
                m if m % 60 == 0 => format!("{} hour{} before", m / 60, if m == 60 { "" } else { "s" }),
                m => format!("{m} minutes before"),
            }).collect::<Vec<_>>().join(", "))
        };
        let response_button = |label: &'static str, response: ResponseStatus, icon_name: &'static str, cx: &mut Context<Self>| {
            let id = id.clone();
            Button::new(SharedString::from(format!("rsvp-{label}")))
                .small()
                .icon(icon(icon_name).size(px(14.)))
                .label(label)
                .toggled(my == Some(response))
                .when(my == Some(response), |b| b.primary())
                .on_click(cx.listener(move |this, _, window, cx| this.respond_to_event(id.clone(), response, window, cx)))
        };
        let description_html = e.description.as_deref().map(|d| if d.contains('<') { sanitize_html(d, RemoteContent::Blocked).html } else { text_to_html(d) });

        v_flex()
            .id("event-detail")
            .size_full()
            .p(px(20.))
            .gap(px(14.))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(10.))
                    .child(div().mt(px(5.)).size(px(12.)).rounded(px(3.)).bg(fill.opacity(1.0)).border_1().border_color(edge))
                    .child(div().flex_1().text_size(px(20.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).child(e.display_title()))
                    .child(Button::new("close-detail").ghost().xsmall().icon(icon("x").size(px(14.))).on_click(cx.listener(|this, _, _w, cx| {
                        this.cal_selected = None;
                        cx.notify();
                    }))),
            )
            .child(v_flex().gap(px(6.)).text_size(px(13.))
                .child(h_flex().gap(px(8.)).items_center().child(icon("clock").size(px(15.)).text_color(p.muted)).child(when))
                .children(e.location.clone().map(|l| h_flex().gap(px(8.)).items_center().child(icon("map-pin").size(px(15.)).text_color(p.muted)).child(div().flex_1().child(l))))
                .children(calendar.map(|c| h_flex().gap(px(8.)).items_center().child(icon("calendar").size(px(15.)).text_color(p.muted)).child(c.name.clone())))
                .children(reminder_text.map(|t| h_flex().gap(px(8.)).items_center().child(icon("bell").size(px(15.)).text_color(p.muted)).child(format!("Reminder {t}"))))
                .when(e.pending, |d| d.child(h_flex().gap(px(8.)).items_center().text_color(p.warning).child(icon("cloud-off").size(px(15.)).text_color(p.warning)).child("Saved on this device, waiting to sync"))))
            .when(e.is_invitation(), |d| {
                d.child(
                    v_flex()
                        .gap(px(8.))
                        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(match my {
                            Some(ResponseStatus::NeedsAction) | None => "Respond".to_owned(),
                            Some(r) => format!("Your response: {}", r.label()),
                        }))
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .child(response_button("Accept", ResponseStatus::Accepted, "check", cx))
                                .child(response_button("Tentative", ResponseStatus::Tentative, "circle-question-mark", cx))
                                .child(response_button("Decline", ResponseStatus::Declined, "x", cx)),
                        ),
                )
            })
            .when(e.conference_url.is_some() || e.html_link.is_some(), |d| {
                let conf = e.conference_url.clone();
                let link = e.html_link.clone();
                d.child(
                    h_flex()
                        .gap(px(6.))
                        .children(conf.map(|url| Button::new("join").small().icon(icon("video").size(px(14.))).label("Join").on_click(move |_, _w, cx| cx.open_url(&url))))
                        .children(link.map(|url| Button::new("open-gcal").ghost().small().icon(icon("external-link").size(px(14.))).label("Open in Google Calendar").on_click(move |_, _w, cx| cx.open_url(&url)))),
                )
            })
            .when(!e.attendees.is_empty(), |d| {
                d.child(
                    v_flex()
                        .gap(px(6.))
                        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(format!("{} guests", e.attendees.len())))
                        .children(e.attendees.iter().take(12).map(|a| {
                            let (ic, color) = match a.response {
                                ResponseStatus::Accepted => ("circle-check", p.success),
                                ResponseStatus::Declined => ("circle-x", p.error),
                                ResponseStatus::Tentative => ("circle-question-mark", p.warning),
                                ResponseStatus::NeedsAction => ("circle", p.muted),
                            };
                            h_flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(12.))
                                .child(icon(ic).size(px(14.)).text_color(color))
                                .child(div().flex_1().truncate().child(format!("{}{}", a.display_name(), if a.is_organizer { " · Organizer" } else if a.optional { " · Optional" } else { "" })))
                        }))
                        .when(e.attendees.len() > 12, |d| d.child(div().text_size(px(11.)).text_color(p.muted).child(format!("and {} more", e.attendees.len() - 12)))),
                )
            })
            .children(description_html.map(|html| div().text_size(px(13.)).line_height(px(20.)).child(TextView::html(SharedString::from(format!("evdesc-{}", e.id)), html).selectable(true))))
            .child(div().flex_1())
            .when(can_edit || can_delete, |d| {
                let edit_id = e.id.clone();
                let del_id = e.id.clone();
                d.child(
                    h_flex()
                        .gap(px(6.))
                        .when(can_edit, |r| r.child(Button::new("edit-event").small().icon(icon("pencil").size(px(14.))).label("Edit").on_click(cx.listener(move |this, _, window, cx| this.start_edit_event(edit_id.clone(), window, cx)))))
                        .when(can_delete, |r| r.child(Button::new("delete-event").ghost().small().icon(icon("trash").size(px(14.)).text_color(p.error)).label("Delete").on_click(cx.listener(move |this, _, window, cx| this.delete_event(del_id.clone(), window, cx))))),
                )
            })
    }

    fn render_new_event(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let Some(form) = self.new_event.as_ref() else { return div().into_any_element() };
        let calendar_name = form.calendar.as_ref().and_then(|id| self.calendars.iter().find(|c| &c.id == id)).map(|c| c.name.clone()).unwrap_or_else(|| "Choose…".into());
        let editable: Vec<(CalendarId, String)> = self.calendars.iter().filter(|c| c.can_edit).map(|c| (c.id.clone(), c.name.clone())).collect();
        let this = cx.entity().downgrade();
        let field = |label: &'static str, input: AnyElement| {
            v_flex().gap(px(4.)).child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(label)).child(input)
        };
        v_flex()
            .id("new-event")
            .size_full()
            .p(px(20.))
            .gap(px(12.))
            .child(h_flex().items_center().child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if form.editing.is_some() { "Edit event" } else { "New event" })).child(Button::new("cancel-new-event").ghost().xsmall().icon(icon("x").size(px(14.))).on_click(cx.listener(|this, _, window, cx| this.cancel_new_event(window, cx)))))
            .child(field("Title", Input::new(&form.title).into_any_element()))
            .child(field("Date", Input::new(&form.date).into_any_element()))
            .child(h_flex().items_center().gap(px(10.)).child(Switch::new("all-day").checked(form.all_day).label("All day").on_click(cx.listener(|this, checked: &bool, _w, cx| {
                if let Some(f) = this.new_event.as_mut() {
                    f.all_day = *checked;
                }
                cx.notify();
            }))))
            .when(!form.all_day, |d| d.child(h_flex().gap(px(10.)).child(div().flex_1().child(field("Start", Input::new(&form.start).into_any_element()))).child(div().flex_1().child(field("End", Input::new(&form.end).into_any_element())))))
            .child(field("Location", Input::new(&form.location).into_any_element()))
            .child(field("Description", div().h(px(120.)).child(Textarea::new(&form.description).h_full()).into_any_element()))
            .when(form.editing.is_none(), |d| d.child(field(
                "Calendar",
                Button::new("pick-calendar")
                    .small()
                    .label(calendar_name)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _window, _cx| {
                        for (id, name) in &editable {
                            let t = this.clone();
                            let id = id.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _w, cx| {
                                let _ = t.update(cx, |this, cx| {
                                    if let Some(f) = this.new_event.as_mut() {
                                        f.calendar = Some(id.clone());
                                    }
                                    cx.notify();
                                });
                            }));
                        }
                        menu
                    })
                    .into_any_element(),
            )))
            .children(form.error.clone().map(|e| h_flex().gap(px(6.)).items_center().text_size(px(12.)).text_color(p.error).child(icon("circle-alert").size(px(14.)).text_color(p.error)).child(e)))
            .child(div().flex_1())
            .child(h_flex().gap(px(8.)).child(Button::new("save-event").primary().label("Save").on_click(cx.listener(|this, _, window, cx| this.save_new_event(window, cx)))).child(Button::new("discard-event").ghost().label("Cancel").on_click(cx.listener(|this, _, window, cx| this.cancel_new_event(window, cx)))))
            .into_any_element()
    }
}

/// Greedy lane assignment for overlapping timed events (sorted by start). Returns
/// `(lane, lanes_in_cluster)` per event.
fn assign_lanes(events: &[&Event]) -> Vec<(usize, usize)> {
    let mut result = vec![(0usize, 1usize); events.len()];
    let mut i = 0;
    while i < events.len() {
        // Find the cluster of transitively overlapping events starting at i.
        let mut cluster_end = events[i].end;
        let mut j = i + 1;
        while j < events.len() && events[j].start < cluster_end {
            cluster_end = cluster_end.max(events[j].end);
            j += 1;
        }
        let mut lane_ends: Vec<i64> = Vec::new();
        for k in i..j {
            let e = events[k];
            let lane = match lane_ends.iter().position(|&end| end <= e.start) {
                Some(l) => {
                    lane_ends[l] = e.end;
                    l
                }
                None => {
                    lane_ends.push(e.end);
                    lane_ends.len() - 1
                }
            };
            result[k].0 = lane;
        }
        for k in i..j {
            result[k].1 = lane_ends.len();
        }
        i = j;
    }
    result
}
