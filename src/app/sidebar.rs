//! 左侧导航栏及其底部设置区。展开 272px；收起为 64px 图标栏（Ctrl+B）。

use gpui::{Animation, AnimationExt};

use super::*;
use crate::ui::tooltip::tooltip;
use crate::ui::widgets::Focusable;

const EXPANDED_WIDTH: f32 = 272.0;
const COLLAPSED_WIDTH: f32 = 64.0;
const ITEM_HEIGHT: f32 = 36.0;
/// macOS 透明标题栏的高度（红绿灯按钮所在的一行，可拖动窗口）。
const MAC_TITLEBAR_HEIGHT: f32 = 28.0;

impl AppState {
    pub(super) fn toggle_advanced(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.advanced_mode = !self.settings.advanced_mode;
        self.save_settings();
        if !self.settings.advanced_mode && matches!(self.page, Page::History | Page::Pool) {
            self.switch_page(Page::Scan, window, cx);
            return;
        }
        self.focus_page_input(self.page, window, cx);
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
        self.sidebar_anim_seq += 1;
        self.theme_panel_open = false;
        self.save_settings();
        cx.notify();
    }

    pub(super) fn open_data_dir(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        storage::open_folder(&self.paths.root);
        self.notify_status(
            format!("已打开 {}", self.paths.root.display()),
            Tone::Muted,
            cx,
        );
    }

    pub(super) fn refresh_history(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history = storage::load_history(&self.paths);
        self.daily_state = storage::load_daily_state(&self.paths);
        self.daily_top_rows = self.daily_state.last_top.clone();
        self.notify_status("历史记录已刷新", Tone::Green, cx);
    }

    /// 侧栏导航项，按 Qoder 设置页侧栏的规格放大：圆角 8 的叠层指示（悬停 sidebar-hover、
    /// 选中 sidebar-active），选中时指示从 0.9 回弹到 1；快捷键提示只在悬停 / 选中时显示。
    /// 收起时只保留图标，名称、计数与快捷键放进 Tooltip。
    pub(super) fn nav_item(
        &self,
        cx: &mut Context<Self>,
        page: Page,
        label: &'static str,
        count: Option<usize>,
        shortcut: u8,
    ) -> gpui::AnyElement {
        let collapsed = self.settings.sidebar_collapsed;
        let item_width = if collapsed { 44.0 } else { 252.0 };
        let active = self.page == page;
        let count = count.filter(|count| *count > 0);
        let group: SharedString = format!("nav-item-{shortcut}").into();
        let indicator = if active {
            div()
                .absolute()
                .inset_0()
                .rounded(px(theme::radius::MD))
                .bg(theme::nav_active())
                .with_animation(
                    ("nav-indicator", self.nav_anim_seq),
                    Animation::new(Duration::from_millis(theme::motion::INDICATOR_MS)),
                    // GPUI 要求缓动结果在 0..=1，回弹只作用在缩放值上。
                    move |indicator, delta| {
                        let scale = 0.9 + 0.1 * theme::motion::overshoot(delta);
                        let shrink = (1.0 - scale) / 2.0;
                        indicator
                            .top(px(shrink * ITEM_HEIGHT))
                            .bottom(px(shrink * ITEM_HEIGHT))
                            .left(px(shrink * item_width))
                            .right(px(shrink * item_width))
                            .opacity((delta * 1.6).min(1.0))
                    },
                )
                .into_any_element()
        } else {
            div()
                .absolute()
                .inset_0()
                .rounded(px(theme::radius::MD))
                .bg(theme::row_hover())
                .opacity(0.)
                .group_hover(group.clone(), |style| style.opacity(1.))
                .into_any_element()
        };
        let shortcut_label: SharedString = if cfg!(target_os = "macos") {
            format!("{}{shortcut}", theme::MOD_KEY)
        } else {
            format!("{} {shortcut}", theme::MOD_KEY)
        }
        .into();
        let icon = widgets::icon(
            page.icon(),
            15.0,
            if active {
                theme::text()
            } else {
                theme::faint()
            },
        );
        let item = div()
            .id(label)
            .group(group.clone())
            .relative()
            .flex()
            .items_center()
            .h(px(ITEM_HEIGHT))
            .rounded(px(theme::radius::MD))
            .cursor_pointer()
            .keyboard_focusable(theme::radius::MD)
            .child(indicator)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.switch_page(page, window, cx);
            }));
        if collapsed {
            let tip = match count {
                Some(count) => format!("{label} · {count}"),
                None => label.to_string(),
            };
            return item
                .w(px(item_width))
                .justify_center()
                .tooltip(tooltip(tip, Some(shortcut_label)))
                .child(icon)
                .into_any_element();
        }
        item.gap_2p5()
            .px_2p5()
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(theme::font::BODY_LG))
                    .font_weight(if active {
                        gpui::FontWeight::MEDIUM
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .child(label),
            )
            .children(count.map(|count| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(18.))
                    .min_w(px(20.))
                    .px_1p5()
                    .rounded(px(theme::radius::SM))
                    .bg(theme::fill_secondary())
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::font::MICRO))
                    .text_color(if active {
                        theme::muted()
                    } else {
                        theme::faint()
                    })
                    .child(count.to_string())
            }))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::font::MICRO))
                    .text_color(theme::quaternary())
                    .opacity(if active { 1. } else { 0. })
                    .group_hover(group, |style| style.opacity(1.))
                    .child(shortcut_label),
            )
            .into_any_element()
    }

    fn nav_group(title: &'static str, collapsed: bool, items: Vec<gpui::AnyElement>) -> gpui::Div {
        let heading = if collapsed {
            div().h(px(1.)).w(px(28.)).my_1().bg(theme::border_subtle())
        } else {
            div()
                .flex()
                .items_center()
                .h(px(20.))
                .mb_1()
                .px_2p5()
                .text_size(px(theme::font::SMALL))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme::quaternary())
                .child(title)
        };
        div()
            .flex()
            .flex_col()
            .when(collapsed, |this| this.items_center())
            .gap(px(2.))
            .child(heading)
            .children(items)
    }

    /// 侧栏底部的一行：图标 / 文字 + 右侧控件，统一 32px 高。
    fn sidebar_row(id: &'static str) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(32.))
            .px_2()
            .rounded(px(theme::radius::MD))
            .text_size(px(theme::font::BODY))
            .text_color(theme::muted())
    }

    /// 收起状态下的图标按钮（也用于展开时标题栏右侧的收起按钮）。
    fn rail_button(
        id: &'static str,
        icon_name: &'static str,
        color: gpui::Rgba,
        tip: impl Into<SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(36.))
            .flex_shrink_0()
            .rounded(px(theme::radius::MD))
            .cursor_pointer()
            .keyboard_focusable(theme::radius::MD)
            .hover(|style| style.bg(theme::row_hover()))
            .tooltip(tooltip(tip, None))
            .child(widgets::icon(icon_name, 14.0, color))
    }

    fn theme_summary(&self) -> String {
        format!(
            "{} · {}",
            theme::current().style().label(),
            if self.settings.follow_system_theme {
                "跟随系统"
            } else {
                theme::current().mode_label()
            }
        )
    }

    /// 侧栏底部的任务状态点；有任务运行时呼吸闪烁。
    fn status_dot(busy: bool, tone: Tone) -> gpui::AnyElement {
        let dot = div()
            .size(px(6.))
            .rounded(px(3.))
            .bg(tone.color())
            .shadow(theme::shadow::halo(tone.color()));
        if !busy {
            return dot.into_any_element();
        }
        dot.with_animation(
            "sidebar-status-pulse",
            Animation::new(Duration::from_millis(1400)).repeat(),
            |dot, delta| {
                dot.opacity(0.3 + 0.7 * (0.5 + 0.5 * (std::f32::consts::TAU * delta).cos()))
            },
        )
        .into_any_element()
    }

    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let collapsed = self.settings.sidebar_collapsed;
        let job = self.job;
        let advanced = self.settings.advanced_mode;
        let busy = job.is_some() || self.aux_job.is_some();
        let (dot_tone, job_label) = if let Some(kind) = job {
            (Tone::Accent, format!("{}运行中", kind.label()))
        } else if let Some(kind) = self.aux_job {
            (Tone::Blue, format!("{}进行中", kind.label()))
        } else {
            (Tone::Green, "空闲".to_string())
        };

        let theme_popover = self.theme_panel_open.then(|| self.render_theme_popover(cx));

        let verified = self
            .verify_records
            .iter()
            .filter(|record| record.ok)
            .count();
        // 一键的自动优选是主入口，放在最前；手动三步按流程顺序排在其后。
        let auto_group = Self::nav_group(
            "自动",
            collapsed,
            vec![self.nav_item(cx, Page::Daily, "自动优选", None, 1)],
        );
        let manual_group = Self::nav_group(
            "手动",
            collapsed,
            vec![
                self.nav_item(cx, Page::Scan, "扫描 IP", Some(self.scan_hits.len()), 2),
                self.nav_item(
                    cx,
                    Page::Speed,
                    "测速排行",
                    Some(self.speed_records.len()),
                    3,
                ),
                self.nav_item(cx, Page::Verify, "可用性验证", Some(verified), 4),
            ],
        );
        let advanced_group = advanced.then(|| {
            Self::nav_group(
                "高级",
                collapsed,
                vec![
                    self.nav_item(
                        cx,
                        Page::Pool,
                        "段池优化",
                        Some(self.pool_segments.len()),
                        5,
                    ),
                    self.nav_item(cx, Page::History, "历史记录", None, 6),
                ],
            )
        });

        // 收起时 Logo 兼作展开按钮：悬停换成侧栏图标。occlude 让点击不落到下面的拖动区。
        let logo = div()
            .id("sidebar-logo")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(36.))
            .flex_shrink_0()
            .rounded(px(theme::radius::MD))
            .bg(theme::accent_dim())
            .child(
                div()
                    .flex()
                    .when(collapsed, |this| {
                        this.group_hover("sidebar-logo", |style| style.opacity(0.))
                    })
                    .child(widgets::icon("logo-mark", 32.0, theme::accent_hover())),
            )
            .when(collapsed, |this| {
                this.group("sidebar-logo")
                    .occlude()
                    .cursor_pointer()
                    .keyboard_focusable(theme::radius::MD)
                    .tooltip(tooltip(
                        format!("展开侧栏 · {}", theme::shortcut("B")),
                        None,
                    ))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sidebar(cx)))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.)
                            .group_hover("sidebar-logo", |style| style.opacity(1.))
                            .child(widgets::icon("panel-left", 15.0, theme::accent_text())),
                    )
            });
        let drag_area = div()
            .id("sidebar-drag")
            .window_control_area(gpui::WindowControlArea::Drag)
            .occlude()
            .flex()
            .items_center()
            .gap_2p5()
            .h_full()
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_mouse_down(MouseButton::Left, cx.listener(Self::start_window_drag))
            })
            .child(logo);
        let header = if collapsed {
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .h(px(60.))
                .child(drag_area)
        } else {
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .h(px(60.))
                .pl_4()
                .pr_2()
                .child(
                    drag_area.flex_1().min_w_0().child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .line_height(px(20.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child("CF Best"),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::font::CAPTION))
                                    .line_height(px(16.))
                                    .text_color(theme::faint())
                                    .child("Cloudflare 优选工作台"),
                            ),
                    ),
                )
                .child(
                    Self::rail_button(
                        "collapse-sidebar",
                        "panel-left",
                        theme::faint(),
                        format!("收起侧栏 · {}", theme::shortcut("B")),
                    )
                    .size(px(30.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sidebar(cx))),
                )
        };

        let nav = div()
            .id("sidebar-nav-scroll")
            .flex()
            .flex_col()
            .gap(px(14.))
            .flex_1()
            .min_h_0()
            .when(!collapsed, |this| this.px_2p5())
            .when(collapsed, |this| this.items_center())
            .pt_1()
            .pb_4()
            .overflow_y_scroll()
            .child(auto_group)
            .child(manual_group)
            .children(advanced_group);

        let footer = if collapsed {
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .mx_2()
                .pt_2()
                .pb_3()
                .border_t_1()
                .border_color(theme::border_subtle())
                .child(
                    div()
                        .id("rail-status")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .tooltip(tooltip(job_label, None))
                        .child(Self::status_dot(busy, dot_tone)),
                )
                .child(
                    Self::rail_button(
                        "toggle-advanced",
                        "sliders",
                        if advanced {
                            theme::accent_text()
                        } else {
                            theme::faint()
                        },
                        if advanced {
                            "高级模式：开"
                        } else {
                            "高级模式：关"
                        },
                    )
                    .on_click(cx.listener(Self::toggle_advanced)),
                )
                .when(crate::tray::SUPPORTED, |this| {
                    this.child(
                        Self::rail_button(
                            "toggle-tray",
                            "archive",
                            if self.settings.tray_enabled {
                                theme::accent_text()
                            } else {
                                theme::faint()
                            },
                            if self.settings.tray_enabled {
                                "托盘常驻：开"
                            } else {
                                "托盘常驻：关"
                            },
                        )
                        .on_click(cx.listener(Self::toggle_tray)),
                    )
                })
                .child(
                    Self::rail_button(
                        "open-theme-panel",
                        if theme::current().is_dark() {
                            "moon"
                        } else {
                            "sun"
                        },
                        theme::faint(),
                        format!("主题：{}", self.theme_summary()),
                    )
                    .on_click(cx.listener(Self::toggle_theme_panel))
                    .children(theme_popover),
                )
                .child(
                    Self::rail_button("open-data-dir", "folder-open", theme::faint(), "数据目录")
                        .on_click(cx.listener(Self::open_data_dir)),
                )
                .child(
                    Self::rail_button(
                        "expand-sidebar",
                        "panel-left",
                        theme::faint(),
                        format!("展开侧栏 · {}", theme::shortcut("B")),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sidebar(cx))),
                )
        } else {
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .mx_3()
                .pt_2()
                .pb_3()
                .border_t_1()
                .border_color(theme::border_subtle())
                .child(
                    Self::sidebar_row("sidebar-status")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Self::status_dot(busy, dot_tone))
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(job_label),
                                ),
                        )
                        .child(widgets::icon("activity", 13.0, theme::quaternary())),
                )
                .child(
                    Self::sidebar_row("toggle-advanced")
                        .cursor_pointer()
                        .keyboard_focusable(theme::radius::MD)
                        .hover(|style| style.bg(theme::row_hover()))
                        .on_click(cx.listener(Self::toggle_advanced))
                        .child("高级模式")
                        .child(widgets::switch(advanced)),
                )
                .when(crate::tray::SUPPORTED, |this| {
                    this.child(
                        Self::sidebar_row("toggle-tray")
                            .cursor_pointer()
                            .keyboard_focusable(theme::radius::MD)
                            .hover(|style| style.bg(theme::row_hover()))
                            .tooltip(tooltip("最小化 / 关闭时隐藏到托盘，后台继续运行", None))
                            .on_click(cx.listener(Self::toggle_tray))
                            .child("托盘常驻")
                            .child(widgets::switch(self.settings.tray_enabled)),
                    )
                })
                .child(
                    Self::sidebar_row("open-theme-panel")
                        .relative()
                        .cursor_pointer()
                        .keyboard_focusable(theme::radius::MD)
                        .hover(|style| style.bg(theme::row_hover()))
                        .on_click(cx.listener(Self::toggle_theme_panel))
                        .child("主题")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .size(px(7.))
                                        .rounded(px(theme::radius::XS))
                                        .bg(theme::accent()),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::font::LABEL))
                                        .text_color(theme::text())
                                        .child(self.theme_summary()),
                                )
                                .child(widgets::icon(
                                    if self.theme_panel_open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    11.0,
                                    theme::faint(),
                                )),
                        )
                        .children(theme_popover),
                )
                .child(
                    Self::sidebar_row("open-data-dir")
                        .justify_start()
                        .cursor_pointer()
                        .keyboard_focusable(theme::radius::MD)
                        .hover(|style| style.bg(theme::row_hover()).text_color(theme::text()))
                        .on_click(cx.listener(Self::open_data_dir))
                        .child(widgets::icon("folder-open", 13.0, theme::faint()))
                        .child("数据目录"),
                )
        };

        let (from, to) = if collapsed {
            (EXPANDED_WIDTH, COLLAPSED_WIDTH)
        } else {
            (COLLAPSED_WIDTH, EXPANDED_WIDTH)
        };
        let sidebar = div()
            .flex()
            .flex_col()
            .w(px(to))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            // macOS 的红绿灯按钮画在窗口左上角，侧栏顶部为它留出一条标题栏高度。
            .when(cfg!(target_os = "macos"), |this| {
                this.child(div().h(px(MAC_TITLEBAR_HEIGHT)).flex_shrink_0())
            })
            .child(header)
            // 导航区占据剩余高度，窗口过矮时滚动而不是挤压顶部 Logo。
            .child(nav)
            .child(footer);
        if self.sidebar_anim_seq == 0 {
            return sidebar.into_any_element();
        }
        sidebar
            .with_animation(
                ("sidebar-width", self.sidebar_anim_seq),
                Animation::new(Duration::from_millis(theme::motion::LAYOUT_MS))
                    .with_easing(theme::motion::ease_out),
                move |sidebar, delta| sidebar.w(px(from + (to - from) * delta)),
            )
            .into_any_element()
    }
}
