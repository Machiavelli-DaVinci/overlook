//! Appearance preference and theme resolution.
//!
//! V1 ships exactly two themes. "Follow system" is a preference that selects one of them;
//! it is not a third theme.

use serde::{Deserialize, Serialize};

/// The user's stored appearance preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AppearancePreference {
    Light,
    Dark,
    #[default]
    FollowSystem,
}

impl AppearancePreference {
    pub const ALL: [AppearancePreference; 3] = [
        AppearancePreference::Light,
        AppearancePreference::Dark,
        AppearancePreference::FollowSystem,
    ];

    /// Label used in Settings and menus.
    pub fn label(self) -> &'static str {
        match self {
            AppearancePreference::Light => "Light",
            AppearancePreference::Dark => "Dark",
            AppearancePreference::FollowSystem => "Follow system",
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            AppearancePreference::Light => "light",
            AppearancePreference::Dark => "dark",
            AppearancePreference::FollowSystem => "follow-system",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "light" => Some(AppearancePreference::Light),
            "dark" => Some(AppearancePreference::Dark),
            "follow-system" | "system" => Some(AppearancePreference::FollowSystem),
            _ => None,
        }
    }

    /// Resolves the preference to one of the two built-in themes.
    pub fn resolve(self, system: SystemAppearance) -> ResolvedTheme {
        match self {
            AppearancePreference::Light => ResolvedTheme::Light,
            AppearancePreference::Dark => ResolvedTheme::Dark,
            AppearancePreference::FollowSystem => match system {
                SystemAppearance::Dark => ResolvedTheme::Dark,
                // Light is the documented fallback when the desktop has no preference or
                // the preference cannot be read.
                SystemAppearance::Light | SystemAppearance::Unknown => ResolvedTheme::Light,
            },
        }
    }
}

/// What the desktop reports through the settings portal, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAppearance {
    Light,
    Dark,
    Unknown,
}

/// One of the two built-in themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

impl ResolvedTheme {
    pub fn is_dark(self) -> bool {
        matches!(self, ResolvedTheme::Dark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_system_is_not_a_third_theme() {
        assert_eq!(
            AppearancePreference::FollowSystem.resolve(SystemAppearance::Dark),
            ResolvedTheme::Dark
        );
        assert_eq!(
            AppearancePreference::FollowSystem.resolve(SystemAppearance::Light),
            ResolvedTheme::Light
        );
        assert_eq!(
            AppearancePreference::FollowSystem.resolve(SystemAppearance::Unknown),
            ResolvedTheme::Light,
            "Light is the fallback"
        );
        assert_eq!(
            AppearancePreference::Dark.resolve(SystemAppearance::Light),
            ResolvedTheme::Dark
        );
    }

    #[test]
    fn keys_round_trip() {
        for pref in AppearancePreference::ALL {
            assert_eq!(AppearancePreference::from_key(pref.as_key()), Some(pref));
        }
    }
}
