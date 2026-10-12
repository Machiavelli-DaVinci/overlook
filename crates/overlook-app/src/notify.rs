//! Desktop notifications through the freedesktop notification service.

use std::time::Duration;

/// Shows a desktop notification without blocking the UI thread. Failures are logged only:
/// the in-app toast is the fallback.
pub fn desktop(summary: String, body: String) {
    std::thread::Builder::new()
        .name("overlook-notify".into())
        .spawn(move || {
            let result = notify_rust::Notification::new()
                .appname("Overlook")
                .summary(&summary)
                .body(&body)
                .icon("x-office-calendar")
                .timeout(Duration::from_secs(20))
                .show();
            if let Err(e) = result {
                tracing::warn!("desktop notification failed: {e}");
            }
        })
        .ok();
}
