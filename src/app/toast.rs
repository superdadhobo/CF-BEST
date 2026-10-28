//! 右下角的轻量提示（复制、导出等瞬时反馈）。

use super::*;

const TOAST_DURATION: Duration = Duration::from_millis(2800);

#[derive(Clone, Debug)]
pub(super) struct Toast {
    id: u64,
    text: String,
    tone: Tone,
    /// 点击提示时打开导出目录。
    opens_exports: bool,
}

impl AppState {
    pub(super) fn show_toast(
        &mut self,
        text: impl Into<String>,
        tone: Tone,
        cx: &mut Context<Self>,
    ) {
        self.push_toast(text.into(), tone, false, cx);
    }

    /// 导出成功的提示，点击可打开导出目录。
    pub(super) fn show_export_toast(
        &mut self,
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) {
        let names = paths
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("、");
        self.notify_status(format!("已导出：{names}"), Tone::Green, cx);
        self.push_toast(
            format!("已导出 {} 个文件 · 点击打开目录", paths.len()),
            Tone::Green,
            true,
            cx,
        );
    }

    fn push_toast(
        &mut self,
        text: String,
        tone: Tone,
        opens_exports: bool,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toast = Some(Toast {
            id,
            text,
            tone,
            opens_exports,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            this.update(cx, |app, cx| {
                if app.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    app.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn copy_text(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.show_toast(format!("已复制{what}"), Tone::Green, cx);
    }

    pub(super) fn render_toast(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let toast = self.toast.clone()?;
        let icon_name = match toast.tone {
            Tone::Red => "x-circle",
            Tone::Amber => "alert-triangle",
            _ => "check-circle",
        };
        Some(
            div()
                .id(("toast", toast.id))
                .absolute()
                .right(px(20.))
                .bottom(px(44.))
                .flex()
                .items_center()
                .gap_2p5()
                .max_w(px(356.))
                .p_3()
                .rounded(px(theme::radius::XL))
                .bg(theme::panel())
                .border_1()
                .border_color(theme::border())
                .shadow(theme::shadow::popover())
                .child(widgets::icon(icon_name, 14.0, toast.tone.color()))
                .child(
                    div()
                        .text_size(px(theme::font::BODY))
                        .text_color(theme::text())
                        .child(toast.text),
                )
                .when(toast.opens_exports, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(theme::row_hover()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            storage::open_folder(&this.paths.exports);
                            this.toast = None;
                            cx.notify();
                        }))
                })
                .into_any_element(),
        )
    }
}
