use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use gpui::{Rgba, rgba};

/// 界面字体。macOS 用系统自带的苹方与 Menlo；其他平台缺少该字体时由系统回退。
#[cfg(target_os = "macos")]
pub const FONT_UI: &str = "PingFang SC";
#[cfg(target_os = "macos")]
pub const FONT_MONO: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
pub const FONT_UI: &str = "Noto Sans SC";
#[cfg(not(target_os = "macos"))]
pub const FONT_MONO: &str = "Cascadia Mono";

/// 快捷键修饰键的显示名，与 GPUI 的 `secondary` 一致：macOS 为 ⌘，其他平台为 Ctrl。
pub const MOD_KEY: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};

/// 快捷键的显示文字，如 `Ctrl+B` / `⌘B`。
pub fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("{MOD_KEY}{key}")
    } else {
        format!("{MOD_KEY}+{key}")
    }
}

/// 界面缩放（Qoder `data-ui-scale=large` 为 1.1×）。所有界面尺寸都经由 [`px`] 缩放，
/// 窗口几何（创建尺寸、最小尺寸）仍直接使用 `gpui::px`。
static UI_SCALE_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

pub const UI_SCALES: [(f32, &str); 4] =
    [(0.9, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%")];

pub fn ui_scale() -> f32 {
    f32::from_bits(UI_SCALE_BITS.load(Ordering::Relaxed))
}

pub fn set_ui_scale(scale: f32) {
    let scale = if scale.is_finite() {
        scale.clamp(0.8, 1.5)
    } else {
        1.0
    };
    UI_SCALE_BITS.store(scale.to_bits(), Ordering::Relaxed);
}

/// 界面像素：按当前界面缩放换算后的 `Pixels`。
pub fn px(value: f32) -> gpui::Pixels {
    gpui::px(value * ui_scale())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeStyle {
    Forest,
    Mint,
    Bee,
    Parchment,
}

impl ThemeStyle {
    pub fn label(self) -> &'static str {
        match self {
            ThemeStyle::Forest => "森林",
            ThemeStyle::Mint => "薄荷",
            ThemeStyle::Bee => "蜜蜂",
            ThemeStyle::Parchment => "羊皮纸",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ThemeStyle::Forest => "forest",
            ThemeStyle::Mint => "mint",
            ThemeStyle::Bee => "bee",
            ThemeStyle::Parchment => "parchment",
        }
    }

    pub fn theme(self, dark: bool) -> Theme {
        match (self, dark) {
            (ThemeStyle::Forest, false) => Theme::ForestLight,
            (ThemeStyle::Forest, true) => Theme::ForestDark,
            (ThemeStyle::Mint, false) => Theme::MintLight,
            (ThemeStyle::Mint, true) => Theme::MintDark,
            (ThemeStyle::Bee, false) => Theme::BeeLight,
            (ThemeStyle::Bee, true) => Theme::BeeDark,
            (ThemeStyle::Parchment, false) => Theme::ParchmentLight,
            (ThemeStyle::Parchment, true) => Theme::ParchmentDark,
        }
    }
}

pub const THEME_STYLES: [ThemeStyle; 4] = [
    ThemeStyle::Forest,
    ThemeStyle::Mint,
    ThemeStyle::Bee,
    ThemeStyle::Parchment,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Theme {
    ForestLight = 0,
    ForestDark = 1,
    MintLight = 2,
    MintDark = 3,
    BeeLight = 4,
    BeeDark = 5,
    ParchmentLight = 6,
    ParchmentDark = 7,
}

impl Theme {
    pub fn from_id(id: &str) -> Self {
        match id.trim().to_ascii_lowercase().as_str() {
            "forest" | "forest-light" => Theme::ForestLight,
            "forest-dark" => Theme::ForestDark,
            "mint" | "mint-light" | "mist-light" => Theme::MintLight,
            "mint-dark" => Theme::MintDark,
            "bee" | "bee-light" => Theme::BeeLight,
            "bee-dark" => Theme::BeeDark,
            "parchment" | "parchment-light" | "light-parchment" | "light" | "warm-light" => {
                Theme::ParchmentLight
            }
            "parchment-dark" | "dark" | "warm-dark" | "graphite-dark" => Theme::ParchmentDark,
            _ => Theme::ParchmentLight,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Theme::ForestLight => "forest-light",
            Theme::ForestDark => "forest-dark",
            Theme::MintLight => "mint-light",
            Theme::MintDark => "mint-dark",
            Theme::BeeLight => "bee-light",
            Theme::BeeDark => "bee-dark",
            Theme::ParchmentLight => "light-parchment",
            Theme::ParchmentDark => "parchment-dark",
        }
    }

    pub fn style(self) -> ThemeStyle {
        match self {
            Theme::ForestLight | Theme::ForestDark => ThemeStyle::Forest,
            Theme::MintLight | Theme::MintDark => ThemeStyle::Mint,
            Theme::BeeLight | Theme::BeeDark => ThemeStyle::Bee,
            Theme::ParchmentLight | Theme::ParchmentDark => ThemeStyle::Parchment,
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(
            self,
            Theme::ForestDark | Theme::MintDark | Theme::BeeDark | Theme::ParchmentDark
        )
    }

    pub fn with_mode(style: ThemeStyle, dark: bool) -> Self {
        style.theme(dark)
    }

    pub fn mode_label(self) -> &'static str {
        if self.is_dark() { "深色" } else { "浅色" }
    }
}

/// 一套主题的全部颜色（RGBA）。数值源自 Qoder Design Kit 的 `tokens.json`，存放在 `theme_tokens.rs`。
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    canvas: u32,
    canvas_end: u32,
    content: u32,
    panel: u32,
    elevated: u32,
    input_bg: u32,
    fill_secondary: u32,
    fill_quaternary: u32,
    text: u32,
    muted: u32,
    faint: u32,
    quaternary: u32,
    on_accent: u32,
    highlight: u32,
    on_highlight: u32,
    border: u32,
    border_strong: u32,
    border_emphasis: u32,
    border_subtle: u32,
    accent: u32,
    accent_hover: u32,
    accent_active: u32,
    accent_text: u32,
    accent_dim: u32,
    accent_soft: u32,
    focus_ring: u32,
    green: u32,
    green_dim: u32,
    red: u32,
    red_dim: u32,
    red_soft: u32,
    amber: u32,
    amber_dim: u32,
    blue: u32,
    blue_dim: u32,
    row_hover: u32,
    nav_active: u32,
    switch_off_bg: u32,
    switch_off_border: u32,
    selection: u32,
    chart_series: u32,
    shadow_2: u32,
    canvas_gradient: bool,
}

#[path = "theme_tokens.rs"]
mod tokens;

static THEME: AtomicU8 = AtomicU8::new(Theme::ParchmentLight as u8);

pub fn set_theme(theme: Theme) {
    THEME.store(theme as u8, Ordering::Relaxed);
}

pub fn current() -> Theme {
    match THEME.load(Ordering::Relaxed) {
        0 => Theme::ForestLight,
        1 => Theme::ForestDark,
        2 => Theme::MintLight,
        3 => Theme::MintDark,
        4 => Theme::BeeLight,
        5 => Theme::BeeDark,
        7 => Theme::ParchmentDark,
        _ => Theme::ParchmentLight,
    }
}

fn palette(theme: Theme) -> &'static Palette {
    match theme {
        Theme::ForestLight => &tokens::FOREST_LIGHT,
        Theme::ForestDark => &tokens::FOREST_DARK,
        Theme::MintLight => &tokens::MINT_LIGHT,
        Theme::MintDark => &tokens::MINT_DARK,
        Theme::BeeLight => &tokens::BEE_LIGHT,
        Theme::BeeDark => &tokens::BEE_DARK,
        Theme::ParchmentLight => &tokens::PARCHMENT_LIGHT,
        Theme::ParchmentDark => &tokens::PARCHMENT_DARK,
    }
}

fn current_palette() -> &'static Palette {
    palette(current())
}

fn color(value: u32) -> Rgba {
    rgba(value)
}

pub struct ThemePreview {
    pub panel: Rgba,
    pub accent: Rgba,
    pub faint: Rgba,
    pub border: Rgba,
}

pub fn preview(style: ThemeStyle, dark: bool) -> ThemePreview {
    let palette = palette(style.theme(dark));
    ThemePreview {
        panel: color(palette.content),
        accent: color(palette.accent),
        faint: color(palette.faint),
        border: color(palette.border),
    }
}

/// 窗口 chrome 背景：纯色或（蜜蜂浅色）180° 渐变，与 Qoder 的 `.q-window` 一致。
pub fn canvas_background() -> gpui::Background {
    canvas_background_of(current_palette())
}

/// 指定风格的 chrome 背景，用于主题预览卡。
pub fn preview_background(style: ThemeStyle, dark: bool) -> gpui::Background {
    canvas_background_of(palette(style.theme(dark)))
}

fn canvas_background_of(palette: &Palette) -> gpui::Background {
    if palette.canvas_gradient {
        gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(color(palette.canvas), 0.),
            gpui::linear_color_stop(color(palette.canvas_end), 1.),
        )
    } else {
        color(palette.canvas).into()
    }
}

macro_rules! palette_accessors {
    ($($(#[$doc:meta])* $name:ident => $field:ident),* $(,)?) => {
        $(
            $(#[$doc])*
            pub fn $name() -> Rgba {
                color(current_palette().$field)
            }
        )*
    };
}

palette_accessors! {
    /// 内容工作区面板（bg-layout）。
    content => content,
    /// 卡片 / 弹层（bg-container）。
    panel => panel,
    /// 卡片内的嵌套分组（bg-elevated）。
    panel_alt => elevated,
    input_bg => input_bg,
    table_head => elevated,
    row_even => fill_quaternary,
    fill_secondary => fill_secondary,
    row_hover => row_hover,
    nav_active => nav_active,
    border => border,
    border_strong => border_strong,
    border_emphasis => border_emphasis,
    /// 面板内部的分隔线（border-quaternary）。
    border_subtle => border_subtle,
    text => text,
    muted => muted,
    faint => faint,
    /// 侧栏分组标题、元信息（text-quaternary）。
    quaternary => quaternary,
    accent => accent,
    accent_hover => accent_hover,
    accent_active => accent_active,
    accent_text => accent_text,
    accent_dim => accent_dim,
    on_accent => on_accent,
    accent_soft => accent_soft,
    /// 键盘焦点外框（primary-border）。
    focus_ring => focus_ring,
    highlight => highlight,
    on_highlight => on_highlight,
    red_soft => red_soft,
    green => green,
    green_dim => green_dim,
    red => red,
    red_dim => red_dim,
    amber => amber,
    amber_dim => amber_dim,
    blue => blue,
    blue_dim => blue_dim,
    switch_off_bg => switch_off_bg,
    switch_off_border => switch_off_border,
    selection => selection,
    /// 图表单系列颜色（Qoder accent-blue，已校验对比度与亮度带）。
    chart_series => chart_series,
    code_bg => elevated,
    shadow_2 => shadow_2,
}

/// 字号刻度（px）。新代码优先使用这些常量，避免出现相近但不一致的字号。
pub mod font {
    pub const MICRO: f32 = 11.0;
    pub const CAPTION: f32 = 12.0;
    pub const SMALL: f32 = 12.5;
    pub const LABEL: f32 = 13.0;
    pub const BODY: f32 = 13.5;
    pub const BODY_LG: f32 = 14.0;
    pub const SUBTITLE: f32 = 15.0;
    pub const TITLE: f32 = 15.5;
}

/// 圆角刻度（px），对应 Qoder 的 sm / md / lg / xl / 2xl。
/// Qoder 在这些圆角上叠加 squircle；GPUI 不支持，按普通圆角绘制。
pub mod radius {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 12.0;
    pub const XXL: f32 = 16.0;
    /// Windows 11 窗口在客户区内的可见圆角：DWM 在客户区外画 1px 边框并按 8px 裁圆角，
    /// 边框内沿约 7px。由系统绘制，不随界面缩放；最大化、贴靠时为直角。
    pub const WINDOW: f32 = 7.0;
}

/// Qoder 的阴影配方（`--q-shadow-border` 等）。GPUI 不支持 inset 阴影，只保留外层。
pub mod shadow {
    use super::px;
    use gpui::{BoxShadow, point};

    fn layer(color: gpui::Rgba, x: f32, y: f32, blur: f32, spread: f32) -> BoxShadow {
        BoxShadow {
            color: color.into(),
            offset: point(px(x), px(y)),
            blur_radius: px(blur),
            spread_radius: px(spread),
        }
    }

    /// `--q-shadow-border`：卡片与外框控件。
    pub fn border() -> Vec<BoxShadow> {
        if super::current().is_dark() {
            vec![layer(gpui::rgba(0xffffff14), 0., 0., 0., 1.)]
        } else {
            vec![
                layer(gpui::rgba(0x0000000f), 0., 0., 0., 1.),
                layer(gpui::rgba(0x0000000f), 0., 1., 2., -1.),
                layer(gpui::rgba(0x0000000a), 0., 2., 4., 0.),
            ]
        }
    }

    /// `--q-shadow-raised` 的外层：抬起的按钮。
    pub fn raised() -> Vec<BoxShadow> {
        vec![layer(super::shadow_2(), 0., 1., 3., 0.)]
    }

    /// 工作区面板的左侧阴影 `-4px 0 12px -8px shadow-color-2`。
    pub fn workbench() -> Vec<BoxShadow> {
        vec![layer(super::shadow_2(), -4., 0., 12., -8.)]
    }

    /// 菜单 / 弹层（`--q-natural-shadow-md`）。
    pub fn popover() -> Vec<BoxShadow> {
        vec![
            layer(gpui::rgba(0x0000001a), 0., 8., 18., -8.),
            layer(gpui::rgba(0x00000014), 0., 18., 42., -20.),
        ]
    }

    /// 输入框聚焦环：`0 0 0 1px border-secondary, 0 0 0 3px border-quaternary`。
    pub fn focus() -> Vec<BoxShadow> {
        vec![
            layer(super::border_subtle(), 0., 0., 0., 3.),
            layer(super::border_strong(), 0., 0., 0., 1.),
        ]
    }

    /// 键盘焦点：Qoder `:focus-visible { outline: 2px solid primary-border; outline-offset: 2px }`。
    pub fn focus_visible() -> Vec<BoxShadow> {
        vec![
            layer(super::focus_ring(), 0., 0., 0., 4.),
            layer(super::content(), 0., 0., 0., 2.),
        ]
    }

    /// 状态圆点的 3px 光圈（`0 0 0 3px color 18%`）。
    pub fn halo(color: gpui::Rgba) -> Vec<BoxShadow> {
        let mut ring = color;
        ring.a = 0.18;
        vec![layer(ring, 0., 0., 0., 3.)]
    }
}

/// 动效时长（ms）与缓动，对应 Qoder 的 fast / 侧栏指示回弹。
pub mod motion {
    pub const INDICATOR_MS: u64 = 260;
    /// Qoder `layout`：侧栏宽度等布局变化。
    pub const LAYOUT_MS: u64 = 220;

    /// `cubic-bezier(.2, .8, .2, 1)` 的近似（ease-out cubic）。
    pub fn ease_out(t: f32) -> f32 {
        1.0 - (1.0 - t.clamp(0., 1.)).powi(3)
    }

    /// `cubic-bezier(.34, 1.4, .64, 1)` 的近似：侧栏选中指示从 0.9 弹到 1。
    pub fn overshoot(t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        let c1 = 1.53;
        let c3 = c1 + 1.0;
        1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
    }
}

#[cfg(test)]
mod tests {
    use super::{px, set_ui_scale, ui_scale};

    #[test]
    fn ui_scale_clamps_and_scales_pixels() {
        set_ui_scale(1.25);
        assert_eq!(px(16.), gpui::px(20.));
        set_ui_scale(9.0);
        assert_eq!(ui_scale(), 1.5);
        set_ui_scale(f32::NAN);
        assert_eq!(ui_scale(), 1.0);
        set_ui_scale(1.0);
    }
}
