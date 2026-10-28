use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icon_table {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8])),*]
    };
}

const ICONS: &[(&str, &[u8])] = icon_table![
    "logo",
    "logo-mark",
    "search",
    "gauge",
    "shield-check",
    "calendar",
    "history",
    "scan",
    "trending-up",
    "badge-check",
    "trophy",
    "archive",
    "target",
    "sun",
    "moon",
    "monitor",
    "panel-left",
    "folder-open",
    "play",
    "stop",
    "refresh",
    "alert-triangle",
    "check-circle",
    "x-circle",
    "terminal",
    "database",
    "sliders",
    "globe",
    "download",
    "copy",
    "chevron-right",
    "chevron-down",
    "activity",
    "clock",
    "layers",
    "sparkles",
    "link",
    "zap",
    "wifi",
    "minimize",
    "maximize",
    "restore",
    "close",
    "key",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let key = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".svg");
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(ICONS
            .iter()
            .filter_map(|(name, _)| name.strip_prefix(&prefix))
            .map(|name| SharedString::from(name.to_string()))
            .collect())
    }
}
