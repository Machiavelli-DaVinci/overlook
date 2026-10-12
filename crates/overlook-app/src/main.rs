//! Overlook — a clear view of what matters.

mod assets;
mod credentials;
mod google_signin;
mod notify;
mod providers;
mod sync;
mod theme;
mod ui;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use overlook_core::time::now;
use overlook_core::*;
use overlook_provider::sample::{sample_mailbox, SampleProvider};
use overlook_store::Store;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

pub const APP_ID: &str = "ai.lastrefuge.Overlook";

static INBOX_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(100);

/// Inbox download size from preferences; set at start-up and whenever it changes.
pub fn store_prefs_inbox_limit() -> usize {
    INBOX_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_inbox_limit(limit: usize) {
    INBOX_LIMIT.store(limit, std::sync::atomic::Ordering::Relaxed);
}
pub const SETTING_APPEARANCE: &str = "appearance";

fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OVERLOOK_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("ai", "Last Refuge", "Overlook")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".overlook"))
}

/// Creates the sample account with the fictional mailbox already downloaded.
pub fn seed_sample_account(store: &Store) -> anyhow::Result<Account> {
    let account = Account {
        id: AccountId::from("sample"),
        provider: ProviderKind::Sample,
        display_name: "Jason Alexander".into(),
        email: "jason@example.com".into(),
        created_at: now(),
    };
    store.upsert_account(&account)?;
    let snapshot = sample_mailbox(&account, now());
    for label in &snapshot.labels {
        store.upsert_label(label)?;
    }
    store.upsert_messages(&snapshot.messages)?;
    store.set_checkpoint(&account.id, snapshot.checkpoint.as_deref())?;
    store.set_sync_state(&account.id, &SyncState::Offline, None)?;
    info!("seeded sample account");
    Ok(account)
}

/// Opens the profile database and makes sure at least one account exists so the shell
/// always has local mail to work with offline.
fn open_store() -> anyhow::Result<(Arc<Store>, Vec<Account>)> {
    let dir = data_dir();
    let store = Store::open(&dir.join("overlook.sqlite"))?;
    let mut accounts = store.accounts()?;
    if accounts.is_empty() {
        accounts.push(seed_sample_account(&store)?);
    }
    Ok((Arc::new(store), accounts))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("overlook=info".parse().unwrap()))
        .init();

    let (store, accounts) = match open_store() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Overlook could not open its local database: {e}");
            std::process::exit(1);
        }
    };
    let provider = SampleProvider::new();
    // The sample account's connection is simulated. Start it connected when asked, so a
    // restart-while-offline test can be followed by a reconnect at launch.
    if std::env::var("OVERLOOK_SAMPLE_ONLINE").is_ok_and(|v| v == "1") {
        provider.set_online(true);
    }

    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let mut fonts: Vec<Cow<'static, [u8]>> = assets::INTER_FACES.iter().map(|f| Cow::Borrowed(*f)).collect();
        fonts.push(Cow::Borrowed(assets::SOURCE_SERIF_DISPLAY));
        if let Err(e) = cx.text_system().add_fonts(fonts) {
            tracing::warn!("bundled fonts could not be loaded: {e}");
        }

        if let Ok(Some(prefs)) = store.setting::<overlook_core::Prefs>("prefs") {
            set_inbox_limit(prefs.inbox_messages as usize);
            overlook_core::tz::set_clock_24h(prefs.clock_24h);
        }
        let preference: AppearancePreference = store
            .setting(SETTING_APPEARANCE)
            .ok()
            .flatten()
            .unwrap_or_default();
        theme::apply_preference(preference, theme::system_appearance(cx.window_appearance()), cx);
        ui::init(cx);

        let bounds = Bounds::centered(None, size(px(1320.), px(920.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some(APP_ID.into()),
            window_min_size: Some(size(px(880.), px(560.))),
            ..TitleBar::window_options()
        };
        let active: AccountId = store
            .setting::<AccountId>("active_account")
            .ok()
            .flatten()
            .filter(|id| accounts.iter().any(|a| &a.id == id))
            .unwrap_or_else(|| accounts[0].id.clone());
        let store = store.clone();
        let accounts = accounts.clone();
        let provider = provider.clone();
        gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| ui::MailWindow::new(store, provider, accounts, active, preference, window, cx))
        })
        .expect("failed to open the Overlook window");
        cx.activate(true);
    });
}
