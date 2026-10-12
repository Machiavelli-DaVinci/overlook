//! Embedded assets: brand artwork and fonts from `branding/`, plus the Lucide icon set
//! bundled with GPUI Kit. Paths under `overlook/` are ours; everything else is delegated.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Static Inter faces (instanced from the bundled variable font) so weights resolve.
pub const INTER_FACES: [&[u8]; 4] = [
    include_bytes!("../../../branding/fonts/static/Inter-Regular.ttf"),
    include_bytes!("../../../branding/fonts/static/Inter-Medium.ttf"),
    include_bytes!("../../../branding/fonts/static/Inter-SemiBold.ttf"),
    include_bytes!("../../../branding/fonts/static/Inter-Bold.ttf"),
];
pub const SOURCE_SERIF_DISPLAY: &[u8] =
    include_bytes!("../../../branding/fonts/SourceSerif4Display-Regular.otf");

const EMBEDDED: &[(&str, &[u8])] = &[
    ("overlook/app-icon.svg", include_bytes!("../../../branding/assets/overlook-app-icon.svg")),
    ("overlook/icon-128.png", include_bytes!("../../../branding/assets/overlook-icon-128.png")),
    ("overlook/icon-64.png", include_bytes!("../../../branding/assets/overlook-icon-64.png")),
    ("overlook/symbolic.svg", include_bytes!("../../../branding/assets/overlook-symbolic.svg")),
    ("overlook/wordmark.svg", include_bytes!("../../../branding/assets/overlook-wordmark-mono.svg")),
    ("overlook/mark-ink.svg", include_bytes!("../../../branding/assets/overlook-mark-ink.svg")),
    ("overlook/alpine-dawn.png", include_bytes!("../../../branding/assets/overlook-alpine-dawn.png")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = EMBEDDED.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut ours: Vec<SharedString> = EMBEDDED
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect();
        ours.extend(gpui_kit::assets::AllAssets.list(path)?);
        Ok(ours)
    }
}

/// Lucide icon by name, e.g. `icon("archive")`.
pub fn icon(name: &str) -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().path(format!("icons/{name}.svg"))
}
