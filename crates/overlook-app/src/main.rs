//! Overlook — a clear view of what matters.

mod assets;
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
pub const SETTING_APPEARANCE: &str = "appearance";

fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OVERLOOK_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("ai", "Last Refuge", "Overlook")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".overlook"))
}

/// Opens the profile database and makes sure a sample account exists so the shell always
/// has realistic local mail to work with offline.
fn open_store() -> anyhow::Result<(Arc<Store>, Account)> {
    let dir = data_dir();
    let store = Store::open(&dir.join("overlook.sqlite"))?;
    let account = match store.accounts()?.into_iter().next() {
        Some(a) => a,
        None => {
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
            info!("seeded sample account in {}", dir.display());
            account
        }
    };
    Ok((Arc::new(store), account))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("overlook=info".parse().unwrap()))
        .init();

    let (store, account) = match open_store() {
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
        if let Err(e) = cx.text_system().add_fonts(vec![
            Cow::Borrowed(assets::INTER_VARIABLE),
            Cow::Borrowed(assets::SOURCE_SERIF_DISPLAY),
        ]) {
            tracing::warn!("bundled fonts could not be loaded: {e}");
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
        let store = store.clone();
        let account = account.clone();
        let provider = provider.clone();
        gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| ui::MailWindow::new(store, provider, account, preference, window, cx))
        })
        .expect("failed to open the Overlook window");
        cx.activate(true);
    });
}
