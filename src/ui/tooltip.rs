//! Qoder Tooltip：最大 240px、圆角 6、12/14px。
//! 浅色主题为 bg-highlight 反色；深色主题为抬起表面 + 1px 边框。

use gpui::{AnyView, App, Context, IntoElement, Render, SharedString, Window, div, prelude::*};

use crate::theme::px;

use crate::theme;

pub struct Tooltip {
    text: SharedString,
    meta: Option<SharedString>,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let dark = theme::current().is_dark();
        div()
            .flex()
            .items_center()
            .gap_2()
            .max_w(px(240.))
            .px_2()
            .py_1p5()
            .rounded(px(theme::radius::SM))
            .font_family(theme::FONT_UI)
            .text_size(px(theme::font::CAPTION))
            .line_height(px(14.))
            .shadow(theme::shadow::popover())
            .when(dark, |this| {
                this.bg(theme::panel_alt())
                    .text_color(theme::text())
                    .border_1()
                    .border_color(theme::border())
            })
            .when(!dark, |this| {
                this.bg(theme::highlight())
                    .text_color(theme::on_highlight())
            })
            .child(self.text.clone())
            .children(self.meta.clone().map(|meta| {
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::font::MICRO))
                    .opacity(0.7)
                    .child(meta)
            }))
    }
}

/// 生成 `.tooltip(...)` 需要的构造函数。
pub fn tooltip(
    text: impl Into<SharedString>,
    meta: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| {
        let text = text.clone();
        let meta = meta.clone();
        cx.new(|_| Tooltip { text, meta }).into()
    }
}
