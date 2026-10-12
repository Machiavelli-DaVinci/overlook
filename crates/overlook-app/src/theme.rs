//! Overlook's two themes, mapped from `branding/tokens.json`.
//!
//! `Palette` holds the semantic tokens the custom mail views paint with. The same values are
//! projected onto the component library's theme so inputs, menus, buttons, and scrollbars
//! match. Switching is instant and touches only colors: entities, focus, drafts, selection,
//! and sync state are untouched.

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Global, Hsla, Rgba, WindowAppearance};
use overlook_core::{AppearancePreference, ResolvedTheme, SystemAppearance};
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Every brand token is defined even where the shell does not use it yet.
pub struct Palette {
    pub resolved: ResolvedTheme,
    pub background: Hsla,
    pub surface: Hsla,
    pub sidebar: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub divider: Hsla,
    pub control_border: Hsla,
    pub hover: Hsla,
    pub accent: Hsla,
    pub action: Hsla,
    pub on_action: Hsla,
    pub selection: Hsla,
    pub selection_text: Hsla,
    pub focus: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub error: Hsla,
    pub brand_accent: Hsla,
    pub avatar: Hsla,
    pub avatar_ink: Hsla,
    pub chip: Hsla,
    pub label_slate: Hsla,
    pub label_plum: Hsla,
    pub label_amber: Hsla,
}

impl Global for Palette {}

fn c(hex: u32) -> Hsla {
    Rgba::from(gpui_kit::rgb(hex)).into()
}

impl Palette {
    pub fn light() -> Self {
        Self {
            resolved: ResolvedTheme::Light,
            background: c(0xF4F3EE),
            surface: c(0xFFFEFB),
            sidebar: c(0xF0F1EC),
            foreground: c(0x192B35),
            muted: c(0x5E6D73),
            divider: c(0xDCE2DF),
            control_border: c(0x74868B),
            hover: c(0xE8EDE9),
            accent: c(0x365F73),
            action: c(0x365F73),
            on_action: c(0xFFFFFF),
            selection: c(0xE3EDEA),
            selection_text: c(0x284D5D),
            focus: c(0x986020),
            success: c(0x386449),
            warning: c(0x805015),
            error: c(0xA13E37),
            brand_accent: c(0xFF9D32),
            avatar: c(0xE4EBE3),
            avatar_ink: c(0x42614E),
            chip: c(0xEAEFEC),
            label_slate: c(0x365F73),
            label_plum: c(0x7C6BB6),
            label_amber: c(0xC98A3A),
        }
    }

    pub fn dark() -> Self {
        Self {
            resolved: ResolvedTheme::Dark,
            background: c(0x101A21),
            surface: c(0x17242C),
            sidebar: c(0x121F27),
            foreground: c(0xF0EBDD),
            muted: c(0xA8B7BA),
            divider: c(0x30424B),
            control_border: c(0x71878D),
            hover: c(0x23353F),
            accent: c(0xA9CDD9),
            action: c(0xA9CDD9),
            on_action: c(0x10232D),
            selection: c(0x28404C),
            selection_text: c(0xDCECEF),
            focus: c(0xFFBA6B),
            success: c(0xA8CEB1),
            warning: c(0xF1BE78),
            error: c(0xFFB0A6),
            brand_accent: c(0xFF9D32),
            avatar: c(0x34483F),
            avatar_ink: c(0xC2D5C3),
            chip: c(0x22323B),
            label_slate: c(0xA9CDD9),
            label_plum: c(0xC5AAFF),
            label_amber: c(0xDBB980),
        }
    }

    pub fn for_theme(theme: ResolvedTheme) -> Self {
        match theme {
            ResolvedTheme::Light => Self::light(),
            ResolvedTheme::Dark => Self::dark(),
        }
    }

    pub fn is_dark(&self) -> bool {
        self.resolved.is_dark()
    }

    /// Color for a user label dot by its token name.
    pub fn label_color(&self, token: Option<&str>) -> Hsla {
        match token {
            Some("plum") => self.label_plum,
            Some("amber") => self.label_amber,
            _ => self.label_slate,
        }
    }
}

pub trait ActivePalette {
    fn palette(&self) -> &Palette;
}

impl ActivePalette for App {
    fn palette(&self) -> &Palette {
        self.global::<Palette>()
    }
}

/// Component-library theme file for one of the two themes.
fn component_theme(theme: ResolvedTheme) -> ThemeConfig {
    let (name, mode, j) = match theme {
        ResolvedTheme::Light => ("Overlook Light", "light", LIGHT_JSON),
        ResolvedTheme::Dark => ("Overlook Dark", "dark", DARK_JSON),
    };
    let json = format!(
        r#"{{"name":"{name}","mode":"{mode}","font.family":"Inter","font.size":14,"radius":6,"radius.lg":10,"shadow":true,"colors":{j}}}"#
    );
    serde_json::from_str(&json).expect("theme json is valid")
}

const LIGHT_JSON: &str = r##"{
  "background": "#F4F3EE", "foreground": "#192B35", "border": "#DCE2DF", "input.border": "#C9D2CF", "ring": "#986020", "caret": "#192B35",
  "primary.background": "#365F73", "primary.foreground": "#FFFFFF", "primary.hover.background": "#2E5366", "primary.active.background": "#264757",
  "secondary.background": "#E8EDE9", "secondary.foreground": "#192B35", "secondary.hover.background": "#DFE6E1", "secondary.active.background": "#D4DDD7",
  "accent.background": "#E3EDEA", "accent.foreground": "#284D5D",
  "muted.background": "#E8EDE9", "muted.foreground": "#5E6D73",
  "popover.background": "#FFFEFB", "popover.foreground": "#192B35",
  "list.background": "#FFFEFB", "list.hover.background": "#E8EDE9", "list.active.background": "#E3EDEA", "list.active.border": "#365F73", "list.even.background": "#FFFEFB", "list.head.background": "#F0F1EC",
  "sidebar.background": "#F0F1EC", "sidebar.foreground": "#192B35", "sidebar.border": "#DCE2DF", "sidebar.accent.background": "#E3EDEA", "sidebar.accent.foreground": "#284D5D", "sidebar.primary.background": "#365F73", "sidebar.primary.foreground": "#FFFFFF",
  "button.background": "#FFFEFB", "button.foreground": "#192B35", "button.hover.background": "#E8EDE9", "button.active.background": "#DFE6E1",
  "button.primary.background": "#365F73", "button.primary.foreground": "#FFFFFF", "button.primary.hover.background": "#2E5366", "button.primary.active.background": "#264757",
  "button.secondary.background": "#E8EDE9", "button.secondary.foreground": "#192B35", "button.secondary.hover.background": "#DFE6E1", "button.secondary.active.background": "#D4DDD7",
  "button.danger.background": "#A13E37", "button.danger.foreground": "#FFFFFF", "button.danger.hover.background": "#8F342E", "button.danger.active.background": "#7B2C27",
  "danger.background": "#A13E37", "danger.foreground": "#FFFFFF", "danger.hover.background": "#8F342E", "danger.active.background": "#7B2C27",
  "success.background": "#386449", "success.foreground": "#FFFFFF", "warning.background": "#805015", "warning.foreground": "#FFFFFF", "info.background": "#365F73", "info.foreground": "#FFFFFF",
  "link": "#365F73", "link.hover": "#2E5366", "link.active": "#264757",
  "selection.background": "#C9DCD6",
  "scrollbar.background": "#00000000", "scrollbar.thumb.background": "#B9C4C0", "scrollbar.thumb.hover.background": "#9DABA7",
  "title_bar.background": "#FFFEFB", "title_bar.border": "#DCE2DF", "status_bar.background": "#F4F3EE", "status_bar.border": "#DCE2DF",
  "tab_bar.background": "#FFFEFB", "tab.background": "#FFFEFB", "tab.foreground": "#5E6D73", "tab.active.background": "#FFFEFB", "tab.active.foreground": "#365F73",
  "switch.background": "#365F73", "switch.thumb.background": "#FFFFFF", "skeleton.background": "#E8EDE9", "overlay": "#101C2466", "window.border": "#DCE2DF",
  "drag.border": "#365F73", "drop_target.background": "#E3EDEA", "progress.bar.background": "#365F73", "slider.background": "#DCE2DF", "slider.thumb.background": "#365F73",
  "accordion.background": "#FFFEFB", "group_box.background": "#FFFEFB", "group_box.foreground": "#192B35",
  "table.background": "#FFFEFB", "table.hover.background": "#E8EDE9", "table.active.background": "#E3EDEA", "table.active.border": "#365F73", "table.even.background": "#FFFEFB", "table.head.background": "#F0F1EC", "table.head.foreground": "#5E6D73", "table.row.border": "#DCE2DF"
}"##;

const DARK_JSON: &str = r##"{
  "background": "#101A21", "foreground": "#F0EBDD", "border": "#30424B", "input.border": "#3B4F59", "ring": "#FFBA6B", "caret": "#F0EBDD",
  "primary.background": "#A9CDD9", "primary.foreground": "#10232D", "primary.hover.background": "#B8D7E1", "primary.active.background": "#98BFCC",
  "secondary.background": "#23353F", "secondary.foreground": "#F0EBDD", "secondary.hover.background": "#2A3F4A", "secondary.active.background": "#1E2E37",
  "accent.background": "#28404C", "accent.foreground": "#DCECEF",
  "muted.background": "#23353F", "muted.foreground": "#A8B7BA",
  "popover.background": "#17242C", "popover.foreground": "#F0EBDD",
  "list.background": "#17242C", "list.hover.background": "#23353F", "list.active.background": "#28404C", "list.active.border": "#A9CDD9", "list.even.background": "#17242C", "list.head.background": "#121F27",
  "sidebar.background": "#121F27", "sidebar.foreground": "#F0EBDD", "sidebar.border": "#30424B", "sidebar.accent.background": "#28404C", "sidebar.accent.foreground": "#DCECEF", "sidebar.primary.background": "#A9CDD9", "sidebar.primary.foreground": "#10232D",
  "button.background": "#17242C", "button.foreground": "#F0EBDD", "button.hover.background": "#23353F", "button.active.background": "#2A3F4A",
  "button.primary.background": "#A9CDD9", "button.primary.foreground": "#10232D", "button.primary.hover.background": "#B8D7E1", "button.primary.active.background": "#98BFCC",
  "button.secondary.background": "#23353F", "button.secondary.foreground": "#F0EBDD", "button.secondary.hover.background": "#2A3F4A", "button.secondary.active.background": "#1E2E37",
  "button.danger.background": "#FFB0A6", "button.danger.foreground": "#10232D", "button.danger.hover.background": "#FFC0B8", "button.danger.active.background": "#F09E94",
  "danger.background": "#FFB0A6", "danger.foreground": "#10232D", "danger.hover.background": "#FFC0B8", "danger.active.background": "#F09E94",
  "success.background": "#A8CEB1", "success.foreground": "#10232D", "warning.background": "#F1BE78", "warning.foreground": "#10232D", "info.background": "#A9CDD9", "info.foreground": "#10232D",
  "link": "#A9CDD9", "link.hover": "#B8D7E1", "link.active": "#98BFCC",
  "selection.background": "#355566",
  "scrollbar.background": "#00000000", "scrollbar.thumb.background": "#3E535D", "scrollbar.thumb.hover.background": "#55707C",
  "title_bar.background": "#17242C", "title_bar.border": "#30424B", "status_bar.background": "#101A21", "status_bar.border": "#30424B",
  "tab_bar.background": "#17242C", "tab.background": "#17242C", "tab.foreground": "#A8B7BA", "tab.active.background": "#17242C", "tab.active.foreground": "#A9CDD9",
  "switch.background": "#A9CDD9", "switch.thumb.background": "#10232D", "skeleton.background": "#23353F", "overlay": "#00000099", "window.border": "#30424B",
  "drag.border": "#A9CDD9", "drop_target.background": "#28404C", "progress.bar.background": "#A9CDD9", "slider.background": "#30424B", "slider.thumb.background": "#A9CDD9",
  "accordion.background": "#17242C", "group_box.background": "#17242C", "group_box.foreground": "#F0EBDD",
  "table.background": "#17242C", "table.hover.background": "#23353F", "table.active.background": "#28404C", "table.active.border": "#A9CDD9", "table.even.background": "#17242C", "table.head.background": "#121F27", "table.head.foreground": "#A8B7BA", "table.row.border": "#30424B"
}"##;

/// Installs both Overlook themes into the component library (once) and applies `theme`.
pub fn apply(theme: ResolvedTheme, cx: &mut App) {
    let light = Rc::new(component_theme(ResolvedTheme::Light));
    let dark = Rc::new(component_theme(ResolvedTheme::Dark));
    Theme::update(cx, |t| {
        t.light_theme = light;
        t.dark_theme = dark;
    });
    let mode = match theme {
        ResolvedTheme::Light => ThemeMode::Light,
        ResolvedTheme::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
    cx.set_global(Palette::for_theme(theme));
    cx.refresh_windows();
}

pub fn system_appearance(appearance: WindowAppearance) -> SystemAppearance {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => SystemAppearance::Dark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => SystemAppearance::Light,
    }
}

/// Resolves and applies the stored preference against the current system appearance.
pub fn apply_preference(pref: AppearancePreference, system: SystemAppearance, cx: &mut App) -> ResolvedTheme {
    let resolved = pref.resolve(system);
    let current = cx.try_global::<Palette>().map(|p| p.resolved);
    if current != Some(resolved) {
        apply(resolved, cx);
    }
    resolved
}
