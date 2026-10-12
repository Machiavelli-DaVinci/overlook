//! The main mail window: header, sidebar, message list, reader, composer, status bar.

mod composer;
mod list;
mod reader;
mod sidebar;
mod widgets;

use crate::assets::icon;
use crate::sync::{run_sync, SyncOutcome};
use crate::theme::{self, ActivePalette, Palette};
use crate::SETTING_APPEARANCE;
pub use composer::Composer;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    h_resizable, resizable_panel, v_resizable, ResizableState, Sizable, TitleBar, WindowExt,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::{h_flex, v_flex};
use overlook_core::time::{format_clock, now};
use overlook_core::*;
use overlook_provider::sample::SampleProvider;
use overlook_store::{ListFilter, MailboxView, Store};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use widgets::*;

actions!(
    overlook,
    [
        Compose,
        FocusSearch,
        Escape,
        SendMessage,
        Reply,
        ReplyAll,
        Forward,
        NextConversation,
        PrevConversation,
        ArchiveSelected,
        TrashSelected,
        ToggleStar,
        ToggleRead,
        OpenSelected,
        SyncNow,
    ]
);

const WINDOW_CONTEXT: &str = "MailWindow";
const LIST_CONTEXT: &str = "MessageList";

/// Stored appearance preference, readable from window-level observers.
pub struct AppearanceGlobal(pub AppearancePreference);
impl Global for AppearanceGlobal {}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-n", Compose, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-e", FocusSearch, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-k", FocusSearch, Some(WINDOW_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-enter", SendMessage, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-r", Reply, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-shift-r", ReplyAll, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-f", Forward, Some(WINDOW_CONTEXT)),
        KeyBinding::new("ctrl-u", ToggleRead, Some(WINDOW_CONTEXT)),
        KeyBinding::new("f5", SyncNow, Some(WINDOW_CONTEXT)),
        KeyBinding::new("j", NextConversation, Some(LIST_CONTEXT)),
        KeyBinding::new("down", NextConversation, Some(LIST_CONTEXT)),
        KeyBinding::new("k", PrevConversation, Some(LIST_CONTEXT)),
        KeyBinding::new("up", PrevConversation, Some(LIST_CONTEXT)),
        KeyBinding::new("e", ArchiveSelected, Some(LIST_CONTEXT)),
        KeyBinding::new("backspace", ArchiveSelected, Some(LIST_CONTEXT)),
        KeyBinding::new("delete", TrashSelected, Some(LIST_CONTEXT)),
        KeyBinding::new("s", ToggleStar, Some(LIST_CONTEXT)),
        KeyBinding::new("u", ToggleRead, Some(LIST_CONTEXT)),
        KeyBinding::new("enter", OpenSelected, Some(LIST_CONTEXT)),
    ]);
}

pub struct Nav {
    pub label: SharedString,
    pub icon: &'static str,
    pub view: MailboxView,
    pub color: Option<Hsla>,
}

impl Nav {
    pub fn view(label: &str, icon: &'static str, view: MailboxView) -> Self {
        Self { label: label.to_owned().into(), icon, view, color: None }
    }
}

pub enum ListRow {
    Group(SharedString, usize),
    Conversation(ConversationSummary),
    Draft(Draft),
}

pub struct MailWindow {
    focus_handle: FocusHandle,
    list_focus: FocusHandle,
    store: Arc<Store>,
    provider: SampleProvider,
    account: Account,
    utc_offset: i32,
    outgoing_dir: PathBuf,

    view: MailboxView,
    filter: ListFilter,
    search_input: Entity<InputState>,
    search_query: String,
    search_coverage: String,

    labels: Vec<Label>,
    conversations: Vec<ConversationSummary>,
    counts: HashMap<MailboxView, usize>,
    drafts: Vec<Draft>,
    outbox: Vec<Draft>,
    status: AccountStatus,

    selected: Option<ConversationId>,
    outbox_selected: Option<DraftId>,
    thread: Vec<Message>,
    expanded: BTreeSet<MessageId>,
    remote_allowed: BTreeSet<MessageId>,

    composer: Option<Composer>,
    appearance: AppearancePreference,
    density: Density,
    reading_pane: ReadingPane,
    sidebar_width: f32,
    list_width: f32,
    list_height: f32,
    panes: Entity<ResizableState>,
    content_panes_h: Entity<ResizableState>,
    content_panes_v: Entity<ResizableState>,

    syncing: bool,
    sync_requested: bool,
    _subscriptions: Vec<Subscription>,
}

impl MailWindow {
    pub fn new(
        store: Arc<Store>,
        provider: SampleProvider,
        account: Account,
        appearance: AppearancePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.set_global(AppearanceGlobal(appearance));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search mail"));
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&search_input, window, |this, state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                let value = state.read(cx).value().to_string();
                this.set_search(value, window, cx);
            }
            InputEvent::PressEnter { .. } => this.list_focus.focus(window, cx),
            _ => {}
        }));
        subs.push(window.observe_window_appearance(|window, cx| {
            let pref = cx.global::<AppearanceGlobal>().0;
            theme::apply_preference(pref, theme::system_appearance(window.appearance()), cx);
        }));

        let density = store.setting::<Density>("density").ok().flatten().unwrap_or_default();
        let reading_pane = store.setting::<ReadingPane>("reading_pane").ok().flatten().unwrap_or_default();
        let sidebar_width = store.setting::<f32>("panes.sidebar").ok().flatten().unwrap_or(232.);
        let list_width = store.setting::<f32>("panes.list").ok().flatten().unwrap_or(340.);
        let list_height = store.setting::<f32>("panes.list_height").ok().flatten().unwrap_or(300.);
        let outgoing_dir = crate::data_dir().join("outgoing");

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            store,
            provider,
            account,
            utc_offset: overlook_core::time::local_utc_offset_minutes(),
            outgoing_dir,
            view: MailboxView::Kind(LabelKind::Inbox),
            filter: ListFilter::All,
            search_input,
            search_query: String::new(),
            search_coverage: String::new(),
            labels: Vec::new(),
            conversations: Vec::new(),
            counts: HashMap::new(),
            drafts: Vec::new(),
            outbox: Vec::new(),
            status: AccountStatus::default(),
            selected: None,
            outbox_selected: None,
            thread: Vec::new(),
            expanded: BTreeSet::new(),
            remote_allowed: BTreeSet::new(),
            composer: None,
            appearance,
            density,
            reading_pane,
            sidebar_width,
            list_width,
            list_height,
            panes: cx.new(|_| ResizableState::default()),
            content_panes_h: cx.new(|_| ResizableState::default()),
            content_panes_v: cx.new(|_| ResizableState::default()),
            syncing: false,
            sync_requested: false,
            _subscriptions: subs,
        };
        // A fresh process has not reached the provider yet: say so until a probe succeeds.
        if !this.provider.is_online() {
            let _ = this.store.set_sync_state(&this.account.id, &SyncState::Offline, None);
        }
        this.reload(cx);
        if this.selected.is_none() {
            if let Some(first) = this.conversations.first().map(|c| c.id.clone()) {
                this.selected = Some(first);
                this.load_thread();
            }
        }
        this.list_focus.focus(window, cx);
        // Synchronize on startup when the provider is reachable; queued work from a previous
        // session is replayed before anything else happens.
        if this.provider.is_online() {
            this.sync_now(window, cx);
        }

        // Periodic synchronization while the account is reachable.
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            let alive = this.update_in(cx, |this, window, cx| {
                if this.provider.is_online() && !this.syncing {
                    this.sync_now(window, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        this
    }

    pub(crate) fn owner(&self) -> Address {
        Address::named(&self.account.display_name, &self.account.email)
    }

    fn view_title(&self) -> String {
        match &self.view {
            MailboxView::Kind(kind) => match kind {
                LabelKind::Inbox => "Inbox",
                LabelKind::Starred => "Starred",
                LabelKind::Drafts => "Drafts",
                LabelKind::Sent => "Sent",
                LabelKind::Archive => "Archive",
                LabelKind::Trash => "Trash",
                LabelKind::Spam => "Spam",
                LabelKind::Outbox => "Outbox",
                LabelKind::User => "Label",
            }
            .to_owned(),
            MailboxView::Label(id) => self.labels.iter().find(|l| &l.id == id).map(|l| l.name.clone()).unwrap_or_default(),
        }
    }

    // ------------------------------------------------------------------ loading

    /// Re-reads everything the window shows from the local store.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let acct = self.account.id.clone();
        let owner = self.account.email.clone();
        self.labels = self.store.labels(&acct).unwrap_or_default();
        self.conversations = if self.search_query.trim().is_empty() {
            self.store.conversations(&acct, &owner, &self.view, self.filter).unwrap_or_default()
        } else {
            self.search_results()
        };
        self.search_coverage = self.store.search_coverage(&acct).map(|c| c.sentence()).unwrap_or_default();
        let mut counts = HashMap::new();
        for view in [MailboxView::Kind(LabelKind::Inbox), MailboxView::Kind(LabelKind::Starred)]
            .into_iter()
            .chain(self.labels.iter().filter(|l| l.kind == LabelKind::User).map(|l| MailboxView::Label(l.id.clone())))
        {
            let n = self.store.view_count(&acct, &owner, &view).unwrap_or(0);
            counts.insert(view, n);
        }
        self.counts = counts;
        self.reload_drafts();
        self.status = self.store.account_status(&acct).unwrap_or_default();
        if let Some(sel) = &self.selected {
            if !self.conversations.iter().any(|c| &c.id == sel) && self.search_query.is_empty() {
                // The selected conversation left this view (archived, trashed, filtered).
                self.selected = None;
                self.thread.clear();
            }
        }
        self.load_thread();
        cx.notify();
    }

    fn reload_drafts(&mut self) {
        let acct = self.account.id.clone();
        self.drafts = self.store.drafts(&acct, false).unwrap_or_default();
        self.outbox = self.store.drafts(&acct, true).unwrap_or_default();
        if let Some(sel) = &self.outbox_selected {
            if !self.outbox.iter().any(|d| &d.id == sel) {
                self.outbox_selected = None;
            }
        }
    }

    fn load_thread(&mut self) {
        self.thread = match &self.selected {
            Some(id) => self.store.messages_in_conversation(&self.account.id, id).unwrap_or_default(),
            None => Vec::new(),
        };
    }

    fn search_results(&self) -> Vec<ConversationSummary> {
        let acct = &self.account.id;
        let hits = self.store.search(acct, &self.search_query).unwrap_or_default();
        if hits.is_empty() {
            return Vec::new();
        }
        let hit_set: BTreeSet<MessageId> = hits.into_iter().collect();
        let all = self.store.messages_for_account(acct).unwrap_or_default();
        let pending = self.store.pending_message_ids(acct).unwrap_or_default();
        let mut by_conv: HashMap<ConversationId, Vec<Message>> = HashMap::new();
        let mut matched: BTreeSet<ConversationId> = BTreeSet::new();
        for m in all {
            if hit_set.contains(&m.id) {
                matched.insert(m.conversation_id.clone());
            }
            by_conv.entry(m.conversation_id.clone()).or_default().push(m);
        }
        let mut out: Vec<ConversationSummary> = matched
            .into_iter()
            .filter_map(|id| ConversationSummary::from_messages(&by_conv[&id], &self.account.email, &pending))
            .filter(|c| match self.filter {
                ListFilter::All => true,
                ListFilter::Unread => c.is_unread(),
                ListFilter::Attachments => c.has_attachments,
            })
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.latest));
        out
    }

    fn selected_summary(&self) -> Option<&ConversationSummary> {
        let id = self.selected.as_ref()?;
        self.conversations.iter().find(|c| &c.id == id)
    }

    // ------------------------------------------------------------ navigation

    fn select_view(&mut self, view: MailboxView, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_query.is_empty() {
            self.search_query.clear();
            let input = self.search_input.clone();
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        if self.composer.as_ref().is_some_and(|c| c.draft.kind != DraftKind::New) {
            self.close_composer(window, cx);
        }
        self.view = view;
        self.outbox_selected = None;
        self.reload(cx);
        if self.selected.is_none() || !self.conversations.iter().any(|c| Some(&c.id) == self.selected.as_ref()) {
            self.selected = self.conversations.first().map(|c| c.id.clone());
            self.expanded.clear();
            self.load_thread();
        }
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    fn set_filter(&mut self, filter: ListFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = filter;
        self.reload(cx);
        self.list_focus.focus(window, cx);
    }

    fn set_search(&mut self, query: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.search_query = query;
        self.reload(cx);
        if !self.search_query.is_empty() && self.selected_summary().is_none() {
            self.selected = self.conversations.first().map(|c| c.id.clone());
            self.load_thread();
        }
        cx.notify();
    }

    fn select_conversation(&mut self, id: Option<ConversationId>, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            self.close_composer(window, cx);
        }
        self.outbox_selected = None;
        self.expanded.clear();
        self.selected = id;
        self.load_thread();
        // Reading marks the conversation read: a local change with its pending operation.
        let unread: Vec<MessageId> = self.thread.iter().filter(|m| !m.is_read).map(|m| m.id.clone()).collect();
        if !unread.is_empty() {
            self.apply_local(overlook_store::LocalChange::SetRead { message_ids: unread, read: true }, window, cx);
        }
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    fn move_selection(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversations.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|id| self.conversations.iter().position(|c| &c.id == id));
        let next = match current {
            Some(i) => (i as i32 + delta).clamp(0, self.conversations.len() as i32 - 1) as usize,
            None => 0,
        };
        let id = self.conversations[next].id.clone();
        if Some(&id) != self.selected.as_ref() {
            self.select_conversation(Some(id), window, cx);
        }
    }

    fn select_outbox_item(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_composer(window, cx);
        self.selected = None;
        self.thread.clear();
        self.outbox_selected = Some(id);
        cx.notify();
    }

    // --------------------------------------------------------- local changes

    fn apply_local(&mut self, change: overlook_store::LocalChange, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.apply_change(&self.account.id, change) {
            Ok(_) => {
                self.reload(cx);
                if self.provider.is_online() {
                    self.sync_now(window, cx);
                }
            }
            Err(e) => window.push_notification(Notification::error(format!("Couldn’t save that change: {e}")), cx),
        }
    }

    fn selected_message_ids(&self) -> Vec<MessageId> {
        self.thread.iter().map(|m| m.id.clone()).collect()
    }

    pub(crate) fn archive(&mut self, ids: Vec<MessageId>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let next = self.neighbour_after_removal();
        self.apply_local(overlook_store::LocalChange::Archive { message_ids: ids }, window, cx);
        self.select_after_removal(next, window, cx);
        window.push_notification(Notification::info("Archived"), cx);
    }

    pub(crate) fn trash(&mut self, ids: Vec<MessageId>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let next = self.neighbour_after_removal();
        self.apply_local(overlook_store::LocalChange::Trash { message_ids: ids }, window, cx);
        self.select_after_removal(next, window, cx);
        window.push_notification(Notification::info("Moved to Trash"), cx);
    }

    fn neighbour_after_removal(&self) -> Option<ConversationId> {
        let i = self.selected.as_ref().and_then(|id| self.conversations.iter().position(|c| &c.id == id))?;
        self.conversations
            .get(i + 1)
            .or_else(|| if i > 0 { self.conversations.get(i - 1) } else { None })
            .map(|c| c.id.clone())
    }

    fn select_after_removal(&mut self, next: Option<ConversationId>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() || !self.conversations.iter().any(|c| Some(&c.id) == self.selected.as_ref()) {
            let next = next.filter(|id| self.conversations.iter().any(|c| &c.id == id));
            self.select_conversation(next, window, cx);
        }
    }

    pub(crate) fn set_read(&mut self, ids: Vec<MessageId>, read: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !ids.is_empty() {
            self.apply_local(overlook_store::LocalChange::SetRead { message_ids: ids, read }, window, cx);
        }
    }

    pub(crate) fn set_starred(&mut self, ids: Vec<MessageId>, starred: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !ids.is_empty() {
            self.apply_local(overlook_store::LocalChange::SetStarred { message_ids: ids, starred }, window, cx);
        }
    }

    // --------------------------------------------------------------- compose

    pub(crate) fn compose_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = Draft::new(self.account.id.clone(), DraftKind::New, now());
        self.outbox_selected = None;
        self.open_composer(draft, window, cx);
    }

    pub(crate) fn reply(&mut self, kind: DraftKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.thread.iter().max_by_key(|m| m.date).cloned() else { return };
        let draft = Draft::in_response_to(self.account.id.clone(), kind, &source, &self.owner(), now());
        if let Err(e) = self.store.save_draft(&draft) {
            window.push_notification(Notification::error(format!("Couldn’t start the draft: {e}")), cx);
            return;
        }
        self.reload_drafts();
        self.open_composer(draft, window, cx);
        if let Some(c) = self.composer.as_mut() {
            c.last_saved = Some(now());
        }
    }

    pub(crate) fn open_draft(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(Some(draft)) = self.store.draft(&id) {
            if let Some(conv) = draft.conversation_id.clone() {
                // Show the conversation being replied to, if it is in this view.
                if self.conversations.iter().any(|c| c.id == conv) {
                    self.selected = Some(conv);
                    self.load_thread();
                }
            }
            self.open_composer(draft, window, cx);
            if let Some(c) = self.composer.as_mut() {
                c.last_saved = Some(now());
            }
        }
    }

    pub(crate) fn cancel_send(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.cancel_send(&id) {
            Ok(draft) => {
                self.outbox_selected = None;
                self.view = MailboxView::Kind(LabelKind::Drafts);
                self.reload(cx);
                self.open_composer(draft, window, cx);
                if let Some(c) = self.composer.as_mut() {
                    c.last_saved = Some(now());
                }
            }
            Err(e) => window.push_notification(Notification::warning(e.to_string()), cx),
        }
    }

    // ------------------------------------------------------------- settings

    fn set_appearance(&mut self, pref: AppearancePreference, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance = pref;
        cx.set_global(AppearanceGlobal(pref));
        if let Err(e) = self.store.set_setting(SETTING_APPEARANCE, &pref) {
            tracing::warn!("could not persist appearance: {e}");
        }
        theme::apply_preference(pref, theme::system_appearance(window.appearance()), cx);
        cx.notify();
    }

    fn toggle_density(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.density = match self.density {
            Density::Comfortable => Density::Compact,
            Density::Compact => Density::Comfortable,
        };
        let _ = self.store.set_setting("density", &self.density);
        cx.notify();
    }

    fn set_reading_pane(&mut self, placement: ReadingPane, cx: &mut Context<Self>) {
        self.reading_pane = placement;
        let _ = self.store.set_setting("reading_pane", &placement);
        cx.notify();
    }

    fn set_online(&mut self, online: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.provider.set_online(online);
        if online {
            self.sync_now(window, cx);
        } else {
            let _ = self.store.set_sync_state(&self.account.id, &SyncState::Offline, None);
            self.reload(cx);
        }
    }

    // ------------------------------------------------------------------ sync

    pub(crate) fn sync_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.syncing {
            self.sync_requested = true;
            return;
        }
        self.syncing = true;
        self.sync_requested = false;
        let store = self.store.clone();
        let provider = self.provider.clone();
        let account = self.account.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let outcome: SyncOutcome = cx.background_spawn(run_sync(store, provider, account)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.syncing = false;
                this.reload(cx);
                for subject in &outcome.sent {
                    window.push_notification(Notification::success(format!("Sent: {subject}")), cx);
                }
                for subject in &outcome.uncertain {
                    window.push_notification(
                        Notification::warning(format!("The connection dropped while sending “{subject}”. Overlook will check whether it arrived before trying again.")),
                        cx,
                    );
                }
                for what in &outcome.failed {
                    window.push_notification(Notification::error(format!("Couldn’t sync: {what}")), cx);
                }
                let more_pending = this.status.pending_operations > 0;
                if this.sync_requested || (more_pending && !outcome.stopped_offline && outcome.applied > 0) {
                    this.sync_now(window, cx);
                }
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------- actions

    fn on_compose(&mut self, _: &Compose, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_new(window, cx);
    }
    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.search_input.clone();
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }
    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            self.close_composer(window, cx);
        } else if !self.search_query.is_empty() {
            let input = self.search_input.clone();
            input.update(cx, |s, cx| s.set_value("", window, cx));
            self.set_search(String::new(), window, cx);
            self.list_focus.focus(window, cx);
        } else {
            self.list_focus.focus(window, cx);
        }
    }
    fn on_send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            self.send_composer(window, cx);
        }
    }
    fn on_reply(&mut self, _: &Reply, window: &mut Window, cx: &mut Context<Self>) {
        self.reply(DraftKind::Reply, window, cx);
    }
    fn on_reply_all(&mut self, _: &ReplyAll, window: &mut Window, cx: &mut Context<Self>) {
        self.reply(DraftKind::ReplyAll, window, cx);
    }
    fn on_forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        self.reply(DraftKind::Forward, window, cx);
    }
    fn on_next(&mut self, _: &NextConversation, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, window, cx);
    }
    fn on_prev(&mut self, _: &PrevConversation, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, window, cx);
    }
    fn on_archive(&mut self, _: &ArchiveSelected, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selected_message_ids();
        self.archive(ids, window, cx);
    }
    fn on_trash(&mut self, _: &TrashSelected, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selected_message_ids();
        self.trash(ids, window, cx);
    }
    fn on_toggle_star(&mut self, _: &ToggleStar, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(summary) = self.selected_summary().cloned() {
            self.set_starred(vec![summary.latest_message_id], !summary.starred, window, cx);
        }
    }
    fn on_toggle_read(&mut self, _: &ToggleRead, window: &mut Window, cx: &mut Context<Self>) {
        let unread_any = self.thread.iter().any(|m| !m.is_read);
        let ids = self.selected_message_ids();
        self.set_read(ids, unread_any, window, cx);
    }
    fn on_open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(latest) = self.thread.iter().max_by_key(|m| m.date) {
            let id = latest.id.clone();
            self.expanded.insert(id);
            cx.notify();
        } else if self.selected.is_none() {
            self.move_selection(1, window, cx);
        }
    }
    fn on_sync_now(&mut self, _: &SyncNow, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_now(window, cx);
    }

    // ---------------------------------------------------------------- header

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let owner = self.owner();
        let appearance = self.appearance;
        let reading_pane = self.reading_pane;
        let density = self.density;
        let this = cx.entity().downgrade();
        let this2 = cx.entity().downgrade();

        let appearance_icon = match p.resolved {
            ResolvedTheme::Light => "sun",
            ResolvedTheme::Dark => "moon",
        };

        TitleBar::new()
            .h(px(64.))
            .bg(p.surface)
            .border_b_1()
            .border_color(p.divider)
            .child(
                h_flex()
                    .size_full()
                    .items_center()
                    .gap(px(20.))
                    .pl(px(10.))
                    .pr(px(8.))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(10.))
                            .w(px(214.))
                            .flex_none()
                            .child(img("overlook/icon-128.png").size(px(30.)).rounded(px(8.)))
                            .child(svg().path("overlook/wordmark.svg").h(px(23.)).w(px(64.)).text_color(p.foreground)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .max_w(px(480.))
                            .h(px(37.))
                            .child(
                                Input::new(&self.search_input)
                                    .prefix(icon("search").size(px(16.)).text_color(p.muted))
                                    .suffix(kbd("Ctrl E", &p))
                                    .cleanable(true),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("appearance-menu")
                            .ghost()
                            .icon(icon(appearance_icon).size(px(17.)))
                            .tooltip(format!("Appearance: {}", appearance.label()))
                            .dropdown_menu(move |menu, window, _cx| {
                                let system = theme::system_appearance(window.appearance());
                                let mut menu = menu.label("Appearance");
                                for pref in AppearancePreference::ALL {
                                    let this = this.clone();
                                    let label = if pref == AppearancePreference::FollowSystem {
                                        match system {
                                            SystemAppearance::Dark => "Follow system (currently Dark)".to_owned(),
                                            SystemAppearance::Light => "Follow system (currently Light)".to_owned(),
                                            SystemAppearance::Unknown => "Follow system".to_owned(),
                                        }
                                    } else {
                                        pref.label().to_owned()
                                    };
                                    menu = menu.item(PopupMenuItem::new(label).checked(pref == appearance).on_click(
                                        move |_, window, cx| {
                                            let _ = this.update(cx, |this, cx| this.set_appearance(pref, window, cx));
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("view-menu")
                            .ghost()
                            .icon(icon("panels-top-left").size(px(17.)))
                            .tooltip("Layout")
                            .dropdown_menu(move |menu, _window, _cx| {
                                let a = this2.clone();
                                let b = this2.clone();
                                let c = this2.clone();
                                menu.label("Reading pane")
                                    .item(PopupMenuItem::new("On the right").checked(reading_pane == ReadingPane::Right).on_click(move |_, _w, cx| {
                                        let _ = a.update(cx, |this, cx| this.set_reading_pane(ReadingPane::Right, cx));
                                    }))
                                    .item(PopupMenuItem::new("Below the list").checked(reading_pane == ReadingPane::Bottom).on_click(move |_, _w, cx| {
                                        let _ = b.update(cx, |this, cx| this.set_reading_pane(ReadingPane::Bottom, cx));
                                    }))
                                    .separator()
                                    .label("Density")
                                    .item(PopupMenuItem::new(if density == Density::Compact { "Comfortable" } else { "Compact" }).on_click(move |_, window, cx| {
                                        let _ = c.update(cx, |this, cx| this.toggle_density(window, cx));
                                    }))
                            }),
                    )
                    .child(owner_avatar(&owner, 29., &p).ml(px(6.)).mr(px(8.))),
            )
    }

    // ------------------------------------------------------------ status bar

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let status = &self.status;
        let online = self.provider.is_online();
        let (dot, state_text) = match &status.state {
            SyncState::UpToDate => (p.success, "Up to date".to_owned()),
            SyncState::Syncing => (p.accent, "Syncing your changes…".to_owned()),
            SyncState::Offline => (p.muted, status.state.sentence(status.pending_operations)),
            SyncState::SignInRequired => (p.warning, status.state.sentence(0)),
            SyncState::SyncFailed(reason) => (p.warning, format!("Sync failed: {reason}")),
        };
        let last = status.last_successful_sync.map(|ts| {
            let (_, t, _) = overlook_core::time::civil(ts, self.utc_offset);
            format!("Last synced {}", format_clock(t))
        });
        let unread: usize = self.conversations.iter().filter(|c| c.is_unread()).count();
        let center = if self.view == MailboxView::Kind(LabelKind::Drafts) {
            format!("{} draft{}", self.drafts.len(), if self.drafts.len() == 1 { "" } else { "s" })
        } else if self.view == MailboxView::Kind(LabelKind::Outbox) {
            format!("{} waiting to send", self.outbox.len())
        } else {
            format!(
                "{} conversation{} · {} unread",
                self.conversations.len(),
                if self.conversations.len() == 1 { "" } else { "s" },
                unread
            )
        };
        let this = cx.entity().downgrade();

        h_flex()
            .items_center()
            .h(px(32.))
            .px(px(14.))
            .gap(px(12.))
            .bg(p.background)
            .border_t_1()
            .border_color(p.divider)
            .text_size(px(12.))
            .text_color(p.muted)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(7.))
                    .min_w(px(260.))
                    .child(div().size(px(7.)).rounded_full().bg(dot))
                    .child(state_text)
                    .when(status.pending_operations > 0 && status.state != SyncState::Offline, |d| {
                        d.child(chip(format!("{} pending", status.pending_operations), &p))
                    })
                    .children(last.map(|l| div().text_color(p.muted).child(format!("· {l}")))),
            )
            .child(div().flex_1())
            .child(center)
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(format!("{} · {}", self.account.provider.label(), self.account.email))
                    .child(
                        Switch::new("connection")
                            .checked(online)
                            .label(if online { "Connected" } else { "Disconnected" })
                            .tooltip("Simulated connection for the sample account")
                            .on_click(cx.listener(|this, checked: &bool, window, cx| this.set_online(*checked, window, cx))),
                    )
                    .child(
                        Button::new("dev-menu")
                            .ghost()
                            .xsmall()
                            .icon(icon("ellipsis").size(px(15.)))
                            .tooltip("Sample account tools")
                            .dropdown_menu(move |menu, _window, _cx| {
                                let a = this.clone();
                                let b = this.clone();
                                menu.label("Sample account")
                                    .item(PopupMenuItem::new("Sync now").on_click(move |_, window, cx| {
                                        let _ = a.update(cx, |this, cx| this.sync_now(window, cx));
                                    }))
                                    .item(PopupMenuItem::new("Drop the connection during the next send").on_click(move |_, window, cx| {
                                        let _ = b.update(cx, |this, cx| {
                                            this.provider.drop_next_send();
                                            window.push_notification(Notification::info("The next send will lose its connection after the server accepts it."), cx);
                                        });
                                    }))
                            }),
                    ),
            )
    }

    // ------------------------------------------------------------------ body

    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.render_sidebar(cx).into_any_element();
        let list = self.render_list(cx).into_any_element();
        let reader = self.render_reader(cx).into_any_element();

        let content = match self.reading_pane {
            ReadingPane::Right => h_resizable("content-h")
                .with_state(&self.content_panes_h)
                .child(resizable_panel().size(px(self.list_width)).size_range(px(260.)..px(640.)).child(list))
                .child(resizable_panel().child(reader))
                .on_resize(cx.listener(|this, state: &Entity<ResizableState>, _window, cx| {
                    if let Some(w) = state.read(cx).sizes().first() {
                        this.list_width = f32::from(*w);
                        let _ = this.store.set_setting("panes.list", &this.list_width);
                    }
                })),
            ReadingPane::Bottom => v_resizable("content-v")
                .with_state(&self.content_panes_v)
                .child(resizable_panel().size(px(self.list_height)).size_range(px(160.)..px(900.)).child(list))
                .child(resizable_panel().child(reader))
                .on_resize(cx.listener(|this, state: &Entity<ResizableState>, _window, cx| {
                    if let Some(h) = state.read(cx).sizes().first() {
                        this.list_height = f32::from(*h);
                        let _ = this.store.set_setting("panes.list_height", &this.list_height);
                    }
                })),
        };

        h_resizable("main")
            .with_state(&self.panes)
            .child(resizable_panel().size(px(self.sidebar_width)).size_range(px(180.)..px(360.)).child(sidebar))
            .child(resizable_panel().child(content))
            .on_resize(cx.listener(|this, state: &Entity<ResizableState>, _window, cx| {
                if let Some(w) = state.read(cx).sizes().first() {
                    this.sidebar_width = f32::from(*w);
                    let _ = this.store.set_setting("panes.sidebar", &this.sidebar_width);
                }
            }))
    }
}

impl Focusable for MailWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MailWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p: Palette = *cx.palette();
        v_flex()
            .id("mail-window")
            .key_context(WINDOW_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_compose))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_send))
            .on_action(cx.listener(Self::on_reply))
            .on_action(cx.listener(Self::on_reply_all))
            .on_action(cx.listener(Self::on_forward))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_archive))
            .on_action(cx.listener(Self::on_trash))
            .on_action(cx.listener(Self::on_toggle_star))
            .on_action(cx.listener(Self::on_toggle_read))
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_sync_now))
            .size_full()
            .bg(p.background)
            .text_color(p.foreground)
            .font_family("Inter")
            .text_size(px(13.))
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
            .child(self.render_status_bar(cx))
    }
}
