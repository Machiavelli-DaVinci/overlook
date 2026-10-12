//! Overlook domain core.
//!
//! This crate is independent of any UI toolkit, database, or network library. It defines
//! the vocabulary the rest of Overlook shares: accounts, labels, folders, messages,
//! conversations, drafts, pending operations, synchronization status, and appearance
//! preferences. It also contains pure logic that must behave identically everywhere:
//! conversation grouping, truthful status wording, HTML sanitization, and theme resolution.

pub mod appearance;
pub mod calendar;
pub mod focus;
pub mod ids;
pub mod model;
pub mod ops;
pub mod sanitize;
pub mod status;
pub mod time;
pub mod tz;

pub use appearance::{AppearancePreference, ResolvedTheme, SystemAppearance};
pub use calendar::*;
pub use focus::{classify_focus, FocusRule, FocusTab};
pub use ids::*;
pub use model::*;
pub use ops::*;
pub use status::*;
