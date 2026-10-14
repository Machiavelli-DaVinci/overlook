//! Privacy-preserving formatting diagnostics. Feed NDJSON objects with an `html`
//! field on stdin. Only aggregate counts are printed; message contents are not.
use overlook_core::sanitize::{clean_only, html_to_text, sanitize_html, RemoteContent};
use std::io::{self, BufRead};
mod support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut messages = 0usize;
    let mut malformed_records = 0usize;
    let mut unstable_cleaning = 0usize;
    let mut unstable_reader = 0usize;
    let mut serialization_only_changes = 0usize;
    let mut folded_history = 0usize;
    let mut blocked_resources = 0usize;
    let mut empty_visible_bodies = 0usize;
    let mut preserved_style_messages = 0usize;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) else {
            malformed_records += 1;
            continue;
        };
        let Some(html) = record.get("html").and_then(|v| v.as_str()) else {
            malformed_records += 1;
            continue;
        };
        messages += 1;
        let clean = clean_only(html, RemoteContent::Blocked);
        let clean_again = clean_only(&clean, RemoteContent::Blocked);
        let cleaning_equal = support::canonical(&clean_again) == support::canonical(&clean);
        unstable_cleaning += usize::from(!cleaning_equal);
        serialization_only_changes += usize::from(cleaning_equal && clean_again != clean);
        let display = sanitize_html(html, RemoteContent::Blocked);
        let again = sanitize_html(&display.html, RemoteContent::Blocked);
        let reader_equal = support::canonical(&again.html) == support::canonical(&display.html);
        unstable_reader += usize::from(!reader_equal);
        serialization_only_changes += usize::from(reader_equal && again.html != display.html);
        folded_history += usize::from(display.quoted.is_some());
        blocked_resources += display.blocked_remote;
        empty_visible_bodies += usize::from(html_to_text(&clean).trim().is_empty());
        preserved_style_messages += usize::from(display.html.contains("style="));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "messages": messages, "malformed_records": malformed_records,
            "unstable_cleaning": unstable_cleaning, "unstable_reader": unstable_reader,
            "serialization_only_changes": serialization_only_changes,
            "folded_history": folded_history, "blocked_resources": blocked_resources,
            "empty_visible_bodies": empty_visible_bodies,
            "preserved_style_messages": preserved_style_messages,
        }))?
    );
    if malformed_records != 0 || unstable_cleaning != 0 || unstable_reader != 0 {
        std::process::exit(1);
    }
    Ok(())
}
