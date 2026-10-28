//! 主题设置面板与主题切换。

use super::*;
use crate::ui::widgets::Focusable;

impl AppState {
    pub(super) fn set_theme(&mut self, next: theme::Theme, cx: &mut Context<Self>) {
        if theme::current() == next {
            return;
        }
        theme::set_theme(next);
        self.settings.theme = next.id().to_string();
        self.save_settings();
        self.notify_theme_entities(cx);
        self.notify_status(
            format!("主题：{} · {}", next.style().label(), next.mode_label()),
            Tone::Muted,
            cx,
        );
    }

    pub(super) fn set_theme_style(&mut self, style: theme::ThemeStyle, cx: &mut Context<Self>) {
        let next = theme::Theme::with_mode(style, theme::current().is_dark());
        self.set_theme(next, cx);
    }

    pub(super) fn set_theme_mode(&mut self, dark: bool, cx: &mut Context<Self>) {
        let next = theme::Theme::with_mode(theme::current().style(), dark);
        self.set_theme(next, cx);
    }

    /// 监听系统明暗变化；在窗口创建后调用一次。
    pub(super) fn watch_system_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_system_appearance(window, cx);
        })
        .detach();
        self.apply_system_appearance(window, cx);
    }

    fn apply_system_appearance(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.settings.follow_system_theme {
            return;
        }
        let dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let next = theme::Theme::with_mode(theme::current().style(), dark);
        if theme::current() != next {
            theme::set_theme(next);
            self.settings.theme = next.id().to_string();
            self.save_settings();
            self.notify_theme_entities(cx);
        }
    }

    /// `mode`：`Some(dark)` 为手动浅色 / 深色，`None` 为跟随系统。
    fn choose_theme_mode(&mut self, mode: Option<bool>, window: &Window, cx: &mut Context<Self>) {
        self.settings.follow_system_theme = mode.is_none();
        self.save_settings();
        match mode {
            Some(dark) => self.set_theme_mode(dark, cx),
            None => {
                self.apply_system_appearance(window, cx);
                self.notify_status("主题明暗已设为跟随系统", Tone::Muted, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_theme_panel(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let just_closed = self
            .theme_panel_closed_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250));
        self.theme_panel_open = !self.theme_panel_open && !just_closed;
        cx.notify();
    }

    /// 设置界面缩放并保存；所有尺寸经 `theme::px` 换算，需要通知输入框重新排版。
    pub(super) fn set_ui_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        theme::set_ui_scale(scale);
        let scale = theme::ui_scale();
        if (self.settings.ui_scale - scale).abs() < f32::EPSILON {
            return;
        }
        self.settings.ui_scale = scale;
        self.save_settings();
        self.notify_theme_entities(cx);
        self.notify_status(format!("界面缩放：{:.0}%", scale * 100.0), Tone::Muted, cx);
    }

    /// 在预设档位间前进 / 后退一档（Ctrl+= / Ctrl+-）。
    pub(super) fn step_ui_scale(&mut self, step: i32, cx: &mut Context<Self>) {
        let current = theme::ui_scale();
        let index = theme::UI_SCALES
            .iter()
            .position(|(scale, _)| (scale - current).abs() < 0.01)
            .unwrap_or(1) as i32;
        let next = (index + step).clamp(0, theme::UI_SCALES.len() as i32 - 1) as usize;
        self.set_ui_scale(theme::UI_SCALES[next].0, cx);
    }

    fn scale_button(
        &self,
        scale: f32,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = (theme::ui_scale() - scale).abs() < 0.01;
        div()
            .id(label)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(px(28.))
            .rounded(px(theme::radius::MD))
            .cursor_pointer()
            .keyboard_focusable(theme::radius::MD)
            .when(active, |this| {
                this.bg(theme::input_bg()).shadow(theme::shadow::raised())
            })
            .font_family(theme::FONT_MONO)
            .text_size(px(theme::font::CAPTION))
            .text_color(if active {
                theme::text()
            } else {
                theme::faint()
            })
            .hover(|style| style.text_color(theme::text()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_ui_scale(scale, cx);
            }))
            .child(label)
            .into_any_element()
    }

    /// 主题设置弹层：浮在「主题」行上方，不参与侧栏布局；点击外部关闭。
    pub(super) fn render_theme_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panel = self
            .render_theme_settings(cx)
            .id("theme-popover")
            .w(px(248.))
            .occlude()
            .shadow(theme::shadow::popover())
            .on_mouse_down_out(cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                this.theme_panel_open = false;
                this.theme_panel_closed_at = Some(Instant::now());
                cx.notify();
            }));
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Corner::BottomLeft)
                        .offset(gpui::point(px(0.), px(-8.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(panel),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    pub(super) fn theme_mode_button(
        &self,
        mode: Option<bool>,
        active: bool,
        icon_name: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id(match mode {
                Some(true) => "theme-mode-dark",
                Some(false) => "theme-mode-light",
                None => "theme-mode-system",
            })
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_1()
            .h(px(28.))
            .px_2()
            .rounded(px(theme::radius::MD))
            .cursor_pointer()
            .keyboard_focusable(theme::radius::MD)
            // Qoder 分段 Tabs：选中段 bg-base + shadow-sm，未选中 text-tertiary。
            .when(active, |this| {
                this.bg(theme::input_bg()).shadow(theme::shadow::raised())
            })
            .text_size(px(theme::font::LABEL))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(if active {
                theme::text()
            } else {
                theme::faint()
            })
            .hover(|style| style.text_color(theme::text()))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.choose_theme_mode(mode, window, cx);
            }))
            .child(widgets::icon(
                icon_name,
                12.0,
                if active {
                    theme::accent_text()
                } else {
                    theme::faint()
                },
            ))
            .child(label)
            .into_any_element()
    }

    pub(super) fn theme_style_card(
        &self,
        style: theme::ThemeStyle,
        dark: bool,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let preview = theme::preview(style, dark);
        div()
            .id(style.id())
            .flex()
            .flex_col()
            .gap_1()
            .w(px(104.))
            .flex_shrink_0()
            .cursor_pointer()
            .keyboard_focusable(theme::radius::MD)
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.set_theme_style(style, cx);
            }))
            .child(
                // Qoder 主题预览：8:5、圆角 8，背景使用该主题真实的 chrome（含渐变）。
                div()
                    .h(px(65.))
                    .w_full()
                    .p_1p5()
                    .rounded(px(theme::radius::MD))
                    .bg(theme::preview_background(style, dark))
                    .border_1()
                    .border_color(if active {
                        preview.accent
                    } else {
                        preview.border
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div().h(px(4.)).flex_1().rounded(px(2.)).bg(preview.accent),
                                    )
                                    .child(div().size(px(7.)).rounded(px(3.5)).bg(preview.panel)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .p_1p5()
                                    .rounded(px(theme::radius::XS))
                                    .bg(preview.panel)
                                    .child(
                                        div()
                                            .h(px(3.))
                                            .w(gpui::relative(0.72))
                                            .rounded(px(1.5))
                                            .bg(preview.faint),
                                    )
                                    .child(
                                        div()
                                            .h(px(3.))
                                            .w(gpui::relative(0.46))
                                            .rounded(px(1.5))
                                            .bg(preview.faint),
                                    )
                                    .child(
                                        div()
                                            .h(px(3.))
                                            .w(gpui::relative(0.6))
                                            .rounded(px(1.5))
                                            .bg(preview.faint),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme::font::SMALL))
                            .font_weight(if active {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::MEDIUM
                            })
                            .text_color(if active {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(style.label()),
                    )
                    .when(active, |row| {
                        row.child(widgets::icon("check-circle", 13.0, theme::accent()))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_theme_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let current = theme::current();
        let dark = current.is_dark();
        let follow = self.settings.follow_system_theme;
        let mode_selector = div()
            .flex()
            .w_full()
            .items_center()
            .gap_1()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(theme::radius::XL))
            .bg(theme::fill_secondary())
            .child(self.theme_mode_button(Some(false), !follow && !dark, "sun", "浅色", cx))
            .child(self.theme_mode_button(Some(true), !follow && dark, "moon", "深色", cx))
            .child(self.theme_mode_button(None, follow, "monitor", "系统", cx));
        let cards = div().flex().flex_wrap().justify_between().gap_2().children(
            theme::THEME_STYLES
                .iter()
                .map(|style| self.theme_style_card(*style, dark, current.style() == *style, cx)),
        );

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2p5()
            .rounded(px(theme::radius::LG))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(theme::font::LABEL))
                            .text_color(theme::muted())
                            .child("主题模式"),
                    )
                    .child(mode_selector),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(theme::font::LABEL))
                                    .text_color(theme::muted())
                                    .child("界面缩放"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(px(theme::font::MICRO))
                                    .text_color(theme::faint())
                                    .child(widgets::kbd(theme::MOD_KEY))
                                    .child(widgets::kbd("+"))
                                    .child(widgets::kbd("-")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(px(2.))
                            .p(px(2.))
                            .rounded(px(theme::radius::XL))
                            .bg(theme::fill_secondary())
                            .children(
                                theme::UI_SCALES
                                    .iter()
                                    .map(|(scale, label)| self.scale_button(*scale, label, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(theme::font::LABEL))
                            .text_color(theme::muted())
                            .child("界面样式"),
                    )
                    .child(cards),
            )
            .child(
                div()
                    .text_size(px(theme::font::SMALL))
                    .line_height(px(19.))
                    .text_color(theme::faint())
                    .child("森林、薄荷、蜜蜂、羊皮纸四套配色"),
            )
    }

    pub(super) fn notify_theme_entities(&self, cx: &mut Context<Self>) {
        self.scan_input.update(cx, |_, cx| cx.notify());
        self.scan_ports.update(cx, |_, cx| cx.notify());
        self.scan_concurrency.update(cx, |_, cx| cx.notify());
        self.scan_timeout.update(cx, |_, cx| cx.notify());
        self.scan_sample.update(cx, |_, cx| cx.notify());
        self.scan_asn.update(cx, |_, cx| cx.notify());
        self.speed_input.update(cx, |_, cx| cx.notify());
        self.speed_coarse.update(cx, |_, cx| cx.notify());
        self.speed_fine.update(cx, |_, cx| cx.notify());
        self.speed_rounds.update(cx, |_, cx| cx.notify());
        self.speed_topn.update(cx, |_, cx| cx.notify());
        self.sub_url.update(cx, |_, cx| cx.notify());
        self.verify_input.update(cx, |_, cx| cx.notify());
        self.daily_segments.update(cx, |_, cx| cx.notify());
        self.daily_test.update(cx, |_, cx| cx.notify());
        self.daily_top.update(cx, |_, cx| cx.notify());
        self.daily_concurrency.update(cx, |_, cx| cx.notify());
        self.daily_auto_time.update(cx, |_, cx| cx.notify());
        self.pool_cursor.update(cx, |_, cx| cx.notify());
        self.probe_samples.update(cx, |_, cx| cx.notify());
        self.probe_concurrency.update(cx, |_, cx| cx.notify());
        self.probe_top.update(cx, |_, cx| cx.notify());
        self.probe_seconds.update(cx, |_, cx| cx.notify());
    }
}
