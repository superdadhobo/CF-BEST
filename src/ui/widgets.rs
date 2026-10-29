use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use gpui::{
    Animation, AnimationExt, Bounds, CursorStyle, Div, ElementId, FontWeight, IntoElement,
    PathBuilder, Pixels, Rgba, SharedString, Stateful, Transformation, Window, canvas, div,
    percentage, point, prelude::*, relative, svg,
};

use crate::theme::px;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
    Soft,
}

pub fn icon(name: &'static str, size: f32, color: Rgba) -> impl IntoElement {
    let size = if name == "logo-mark" {
        size
    } else {
        size * 1.18
    };
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}

/// 加载指示：淡色整圈轨道上一段圆头弧线匀速旋转。
///
/// 不用旋转 SVG：gpui 绘制 SVG 时会把位置取整到像素，旋转中心却按未取整的边界计算，
/// 图标每圈都会偏移晃动。这里每帧按边界的精确圆心重新生成矢量路径，始终绕中心转。
pub fn spinner(id: &'static str, size: f32, color: Rgba) -> impl IntoElement {
    div().flex_shrink_0().size(px(size * 1.18)).with_animation(
        id,
        Animation::new(Duration::from_millis(SPINNER_TURN_MS)).repeat(),
        move |this, delta| {
            this.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_spinner(bounds, delta, color, window),
                )
                .size_full(),
            )
        },
    )
}

const SPINNER_TURN_MS: u64 = 850;
/// 弧线占整圈的比例。
const SPINNER_ARC: f32 = 0.28;
const SPINNER_TRACK_ALPHA: f32 = 0.2;

fn paint_spinner(bounds: Bounds<Pixels>, turn: f32, color: Rgba, window: &mut Window) {
    let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
    if side <= 0.0 {
        return;
    }
    let center = bounds.center();
    let (cx, cy) = (f32::from(center.x), f32::from(center.y));
    let stroke = (side * 0.11).max(1.5);
    let radius = side * 0.375;
    let at = |r: f32, angle: f32| {
        point(
            gpui::px(cx + r * angle.cos()),
            gpui::px(cy + r * angle.sin()),
        )
    };
    let radii = |r: f32| point(gpui::px(r), gpui::px(r));

    let mut track = PathBuilder::stroke(gpui::px(stroke));
    track.move_to(at(radius, 0.0));
    track.arc_to(radii(radius), gpui::px(0.), false, true, at(radius, PI));
    track.arc_to(radii(radius), gpui::px(0.), false, true, at(radius, 0.0));
    track.close();
    if let Ok(path) = track.build() {
        window.paint_path(
            path,
            Rgba {
                a: color.a * SPINNER_TRACK_ALPHA,
                ..color
            },
        );
    }

    // 圆头弧线按填充形状绘制：外弧 → 末端半圆 → 内弧 → 起端半圆。
    let start = TAU * turn - FRAC_PI_2;
    let end = start + TAU * SPINNER_ARC;
    let (outer, inner, cap) = (radius + stroke / 2.0, radius - stroke / 2.0, stroke / 2.0);
    let large = SPINNER_ARC > 0.5;
    let mut arc = PathBuilder::fill();
    arc.move_to(at(outer, start));
    arc.arc_to(radii(outer), gpui::px(0.), large, true, at(outer, end));
    arc.arc_to(radii(cap), gpui::px(0.), false, true, at(inner, end));
    arc.arc_to(radii(inner), gpui::px(0.), large, false, at(inner, start));
    arc.arc_to(radii(cap), gpui::px(0.), false, true, at(outer, start));
    arc.close();
    if let Ok(path) = arc.build() {
        window.paint_path(path, color);
    }
}

static KEYBOARD_NAV: AtomicBool = AtomicBool::new(false);

/// 是否处于键盘导航中（按过 Tab 且之后没有点击鼠标），对应 CSS `:focus-visible`。
pub fn keyboard_nav() -> bool {
    KEYBOARD_NAV.load(Ordering::Relaxed)
}

pub fn set_keyboard_nav(active: bool) -> bool {
    KEYBOARD_NAV.swap(active, Ordering::Relaxed) != active
}

/// 可通过 Tab 聚焦的交互元素：Enter / 空格触发点击，键盘导航时显示焦点外框。
pub trait Focusable: InteractiveElement + Styled + Sized {
    fn keyboard_focusable(self, radius: f32) -> Self {
        let this = self.tab_index(0);
        if keyboard_nav() {
            this.focus(move |style| {
                style
                    .rounded(px(radius))
                    .shadow(theme::shadow::focus_visible())
            })
        } else {
            this
        }
    }
}

impl<T: InteractiveElement + Styled> Focusable for T {}

/// 带图标的按钮。`id` 让按钮成为有状态元素，从而拥有按下态（Qoder `primary-active`）
/// 与 Tab 焦点；键盘聚焦时同时呈现悬停底色，与 Qoder 的 `:focus-visible` 一致。
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon_name: &'static str,
    kind: ButtonKind,
) -> Stateful<Div> {
    button_with_state(id, label, icon_name, kind, false)
}

/// 同 [`button`]，`disabled` 时降低不透明度、显示「不可用」光标，且不响应悬停 / 按下。
/// 点击回调仍会触发，由回调内部的状态检查给出提示。
/// （GPUI 不允许对同一元素设置两次 hover，所以禁用态必须在构造时决定。）
pub fn button_with_state(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon_name: &'static str,
    kind: ButtonKind,
    disabled: bool,
) -> Stateful<Div> {
    let (hover, pressed) = match kind {
        ButtonKind::Primary => (theme::accent_hover(), theme::accent_active()),
        ButtonKind::Secondary | ButtonKind::Ghost => (theme::fill_secondary(), theme::nav_active()),
        ButtonKind::Danger => (theme::red_soft(), theme::red_soft()),
        ButtonKind::Soft => (theme::accent_soft(), theme::accent_soft()),
    };
    let button = button_inner(label.into(), Some(icon_name), kind)
        .id(id)
        .tab_index(0);
    if disabled {
        return button
            .opacity(0.45)
            .cursor(CursorStyle::OperationNotAllowed);
    }
    let button = button
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed));
    if keyboard_nav() {
        button.focus(move |style| style.bg(hover).shadow(theme::shadow::focus_visible()))
    } else {
        button
    }
}

/// 快捷键键帽（Qoder Kbd）：高 20、圆角 4、11px，bg-highlight 上的反色文字。
pub fn kbd(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(18.))
        .min_w(px(18.))
        .px(px(5.))
        .rounded(px(theme::radius::XS))
        .bg(theme::highlight())
        .font_family(theme::FONT_MONO)
        .text_size(px(theme::font::MICRO))
        .text_color(theme::on_highlight())
        .child(text.into())
}

/// Ghost 按钮的底色：悬停色 fill-secondary 的 60%。浅色主题里它是半透明黑、深色主题里是实色，
/// 按比例取淡才能在所有主题下都比悬停浅一级，并且放在卡片或嵌套分组上都看得出来。
fn ghost_fill() -> Rgba {
    let mut fill = theme::fill_secondary();
    fill.a *= 0.6;
    fill
}

fn button_inner(label: SharedString, icon_name: Option<&'static str>, kind: ButtonKind) -> Div {
    let (bg, fg, border) = match kind {
        ButtonKind::Primary => (theme::accent(), theme::on_accent(), theme::accent()),
        // Qoder 的 outline 变体：bg-base + border-tertiary，悬停 fill-secondary。
        ButtonKind::Secondary => (theme::input_bg(), theme::text(), theme::border()),
        // 文字按钮也铺一层淡底色，只有文字时显得突兀；悬停时加深为 fill-secondary。
        ButtonKind::Ghost => (ghost_fill(), theme::muted(), gpui::rgba(0x00000000)),
        ButtonKind::Danger => (theme::red_dim(), theme::red(), theme::red_dim()),
        ButtonKind::Soft => (
            theme::accent_dim(),
            theme::accent_text(),
            theme::accent_dim(),
        ),
    };

    // Qoder Button `lg`：h-9、gap-2、px-4、14px medium；squircle-2xl 近似为 12px 圆角。
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .h(px(36.))
        .px_4()
        .rounded(px(theme::radius::XL))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_size(px(theme::font::BODY_LG))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .children(icon_name.map(|name| icon(name, 14.0, fg)))
        .child(label)
}

/// 卡片：bg-container + Qoder `--q-shadow-border`（1px 环形阴影代替描边）。
pub fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p(px(16.))
        .rounded(px(theme::radius::XL))
        .bg(theme::panel())
        .shadow(theme::shadow::border())
}

pub fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::font::LABEL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::muted())
        .child(text.into())
}

pub fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::font::BODY))
        .line_height(px(21.))
        .text_color(theme::faint())
        .child(text.into())
}

pub fn field(name: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .child(label(name))
        .child(control)
}

pub fn section_title(icon_name: &'static str, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(icon(icon_name, 14.0, theme::muted()))
        .child(
            div()
                .text_size(px(theme::font::TITLE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(title.into()),
        )
}

pub fn inline_guide(steps: [&'static str; 3]) -> Div {
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(icon("sparkles", 12.0, theme::faint()))
                .child(
                    div()
                        .text_size(px(theme::font::SMALL))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::faint())
                        .child("三步开始"),
                ),
        )
        .child(div().w(px(1.)).h(px(12.)).bg(theme::border()));
    for (index, label) in steps.iter().enumerate() {
        if index > 0 {
            row = row.child(icon("chevron-right", 11.0, theme::faint()));
        }
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(15.))
                        .rounded(px(theme::radius::MD))
                        .bg(theme::row_hover())
                        .font_family(theme::FONT_MONO)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .text_size(px(theme::font::LABEL))
                        .text_color(theme::muted())
                        .child(*label),
                ),
        );
    }
    row
}

#[derive(Clone, Copy, Debug)]
pub enum Tone {
    Accent,
    Green,
    Red,
    Amber,
    Blue,
    Muted,
}

impl Tone {
    pub fn color(self) -> Rgba {
        match self {
            Tone::Accent => theme::accent_text(),
            Tone::Green => theme::green(),
            Tone::Red => theme::red(),
            Tone::Amber => theme::amber(),
            Tone::Blue => theme::blue(),
            Tone::Muted => theme::muted(),
        }
    }

    pub fn bg(self) -> Rgba {
        match self {
            Tone::Accent => theme::accent_dim(),
            Tone::Green => theme::green_dim(),
            Tone::Red => theme::red_dim(),
            Tone::Amber => theme::amber_dim(),
            Tone::Blue => theme::blue_dim(),
            Tone::Muted => theme::row_hover(),
        }
    }
}

pub fn pill(text: impl Into<SharedString>, tone: Tone) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(24.))
        .px_2()
        .rounded(px(theme::radius::SM))
        .bg(tone.bg())
        .text_color(tone.color())
        .text_size(px(theme::font::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

pub fn running_chip(id: &'static str, label: impl Into<SharedString>, tone: Tone) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(36.))
        .px_3()
        .rounded(px(theme::radius::MD))
        .bg(tone.bg())
        .text_color(tone.color())
        .text_size(px(theme::font::BODY))
        .font_weight(FontWeight::MEDIUM)
        .child(spinner(id, 12.0, tone.color()))
        .child(label.into())
}

pub fn pill_action(text: impl Into<SharedString>, tone: Tone) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(36.))
        .px_2p5()
        .rounded(px(theme::radius::MD))
        .bg(tone.bg())
        .text_color(tone.color())
        .text_size(px(theme::font::BODY))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

pub fn stat(name: impl Into<SharedString>, value: impl Into<SharedString>, tone: Tone) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w(px(84.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(div().size(px(5.)).rounded(px(3.)).bg(tone.color()))
                .child(
                    div()
                        .text_size(px(theme::font::SMALL))
                        .text_color(theme::faint())
                        .child(name.into()),
                ),
        )
        .child(
            div()
                .text_size(px(17.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(value.into()),
        )
}

fn progress_track(fraction: f32, shimmer: Option<f32>) -> Div {
    let fraction = fraction.clamp(0., 1.);
    let indeterminate = fraction <= 0.001 && shimmer.is_some();
    let width = if indeterminate { 0.24 } else { fraction };
    let mut fill = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .h_full()
        .w(relative(width))
        .rounded(px(2.))
        .overflow_hidden()
        .bg(theme::accent());
    if let Some(delta) = shimmer {
        fill = fill
            .child(
                div()
                    .h_full()
                    .w(relative(0.04 + 0.78 * delta))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .h_full()
                    .w(relative(0.2))
                    .flex_shrink_0()
                    .bg(gpui::rgba(0xffffff4d)),
            );
    }
    let mut track = div()
        .flex()
        .items_center()
        .w_full()
        .h(px(4.))
        .rounded(px(2.))
        .bg(theme::row_hover())
        .overflow_hidden();
    if indeterminate && let Some(delta) = shimmer {
        track = track.child(div().h_full().w(relative(0.76 * delta)).flex_shrink_0());
    }
    track.child(fill)
}

pub fn progress_bar(id: &'static str, fraction: f32, active: bool) -> Div {
    let fraction = fraction.clamp(0., 1.);
    if !active && fraction <= 0.001 {
        return div()
            .w_full()
            .h(px(6.))
            .rounded(px(3.))
            .bg(theme::row_hover());
    }
    let mut wrapper = div().w_full().h(px(6.));
    if active {
        wrapper = wrapper.child(div().w_full().with_animation(
            id,
            Animation::new(Duration::from_millis(1400)).repeat(),
            move |_, delta| progress_track(fraction, Some(delta)),
        ));
    } else {
        wrapper = wrapper.child(progress_track(fraction, None));
    }
    wrapper
}

pub fn progress_caption(
    label: impl Into<SharedString>,
    active: bool,
    spinner_id: &'static str,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .when(active, |this| {
            this.child(spinner(spinner_id, 12.0, theme::accent()))
        })
        .child(
            div()
                .text_size(px(theme::font::BODY))
                .line_height(px(21.))
                .text_color(theme::muted())
                .child(label.into()),
        )
}

pub fn table_header(columns: &[(&'static str, f32)]) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(38.))
        .px_4()
        .bg(theme::table_head())
        .border_b_1()
        .border_color(theme::border())
        .children(columns.iter().map(|(name, width)| {
            div()
                .w(px(*width))
                .flex_shrink_0()
                .text_size(px(theme::font::SMALL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(*name)
        }))
}

pub fn empty_state(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .size_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(52.))
                .mb_1()
                .rounded(px(14.))
                .bg(theme::panel_alt())
                .border_1()
                .border_color(theme::border())
                .child(icon(icon_name, 22.0, theme::faint())),
        )
        .child(
            div()
                .text_size(px(theme::font::SUBTITLE))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_size(px(theme::font::BODY))
                .line_height(px(21.))
                .text_color(theme::faint())
                .child(detail.into()),
        )
}

/// Qoder Switch MD：32×20 圆角轨道、16px 圆形 thumb，关 1px / 开 13px 位移。
pub fn switch(active: bool) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .w(px(32.))
        .h(px(20.))
        .rounded(px(10.))
        .border_1()
        .border_color(if active {
            gpui::rgba(0x00000000)
        } else {
            theme::switch_off_border()
        })
        .bg(if active {
            theme::accent()
        } else {
            theme::switch_off_bg()
        })
        .child(
            div()
                .size(px(16.))
                .rounded(px(8.))
                .ml(if active { px(13.) } else { px(1.) })
                .bg(gpui::rgb(0xffffff))
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::rgba(0x00000026).into(),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                }]),
        )
}

pub fn log_view(lines: &[String]) -> impl IntoElement {
    div()
        .id("log-view")
        .flex()
        .flex_col()
        .gap_0p5()
        .h(px(220.))
        .w_full()
        .p_3()
        .rounded(px(theme::radius::MD))
        .bg(theme::code_bg())
        .border_1()
        .border_color(theme::border_subtle())
        .overflow_y_scroll()
        .children(lines.iter().map(|line| {
            let (color, marker) = log_line_tone(line);
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .mt(px(4.))
                        .w(px(3.))
                        .h(px(11.))
                        .flex_shrink_0()
                        .rounded(px(2.))
                        .bg(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(theme::font::SMALL))
                        .line_height(px(19.))
                        .text_color(color)
                        .child(line.clone()),
                )
        }))
}

fn log_line_tone(line: &str) -> (Rgba, Rgba) {
    const TRANSPARENT: u32 = 0x00000000;
    if line.contains("失败") || line.contains("错误") || line.contains("ERR") {
        (theme::red(), theme::red())
    } else if line.contains("警告")
        || line.contains("超时")
        || line.contains("跳过")
        || line.contains("重试")
        || line.contains("WARN")
    {
        (theme::amber(), theme::amber())
    } else if line.contains("通过") || line.contains("完成") || line.contains("已保存") {
        (theme::green(), theme::green())
    } else if line.starts_with('[') {
        (theme::muted(), theme::border_strong())
    } else {
        (theme::faint(), gpui::rgba(TRANSPARENT))
    }
}

pub fn badge_dot(tone: Tone) -> Div {
    div().size(px(6.)).rounded(px(3.)).bg(tone.color())
}

/// 行内复制提示：平时隐藏，悬停在名为 `group` 的行上时显示。
pub fn copy_hint(group: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .child(icon("copy", 12.0, theme::faint()))
}

/// 可排序表头单元格。`sort` 为 `None` 表示未按此列排序，`Some(true)` 降序、`Some(false)` 升序。
pub fn sortable_header_cell(name: &'static str, sort: Option<bool>) -> Div {
    let active = sort.is_some();
    div()
        .keyboard_focusable(theme::radius::XS)
        .flex()
        .items_center()
        .gap_1()
        .h_full()
        .cursor_pointer()
        .text_size(px(theme::font::SMALL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|style| style.text_color(theme::text()))
        .child(name)
        .child(match sort {
            Some(true) => icon("chevron-down", 10.0, theme::accent_text()).into_any_element(),
            Some(false) => svg()
                .path("icons/chevron-down.svg")
                .size(px(11.8))
                .text_color(theme::accent_text())
                .with_transformation(Transformation::rotate(percentage(0.5)))
                .into_any_element(),
            None => div()
                .opacity(0.)
                .group_hover("sort-header", |style| style.opacity(0.5))
                .child(icon("chevron-down", 10.0, theme::faint()))
                .into_any_element(),
        })
}

/// 表头容器，供 `sortable_header_cell` 与普通列混排。
pub fn table_header_row() -> Div {
    div()
        .group("sort-header")
        .flex()
        .items_center()
        .h(px(38.))
        .px_4()
        .bg(theme::table_head())
        .border_b_1()
        .border_color(theme::border())
}

pub fn plain_header_cell(name: &'static str) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(px(theme::font::SMALL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::muted())
        .child(name)
}
