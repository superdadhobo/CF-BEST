//! 可拖拽调整的表格列宽：表头单元格右侧的拖拽柄，宽度持久化到设置。
//! 每张表最后一列自适应剩余宽度，不提供拖拽柄。

use gpui::MouseMoveEvent;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableId {
    Scan,
    Speed,
    Verify,
}

impl TableId {
    /// 可调整列（不含最后一列）的默认宽度。
    pub fn defaults(self) -> &'static [f32] {
        match self {
            TableId::Scan => &[220., 70., 100., 80., 140.],
            TableId::Speed => &[200., 90., 100., 100., 100., 80.],
            TableId::Verify => &[190., 80., 80., 90., 76.],
        }
    }

    fn key(self) -> &'static str {
        match self {
            TableId::Scan => "scan",
            TableId::Speed => "speed",
            TableId::Verify => "verify",
        }
    }
}

pub const MIN_WIDTH: f32 = 56.0;
pub const MAX_WIDTH: f32 = 480.0;

#[derive(Clone, Copy, Debug)]
pub struct ColumnDrag {
    table: TableId,
    column: usize,
    start_x: gpui::Pixels,
    start_width: f32,
}

/// 拖拽后的新宽度：位移按界面缩放换回设计尺寸，并限制在合理范围。
pub fn dragged_width(start_width: f32, delta_px: f32, scale: f32) -> f32 {
    (start_width + delta_px / scale.max(0.1)).clamp(MIN_WIDTH, MAX_WIDTH)
}

impl AppState {
    /// 当前列宽（未保存或列数不匹配时使用默认值）。
    pub(super) fn column_widths(&self, table: TableId) -> Vec<f32> {
        let defaults = table.defaults();
        match self.settings.column_widths.get(table.key()) {
            Some(saved) if saved.len() == defaults.len() => saved.clone(),
            _ => defaults.to_vec(),
        }
    }

    fn set_column_width(&mut self, table: TableId, column: usize, width: f32) {
        let mut widths = self.column_widths(table);
        if let Some(slot) = widths.get_mut(column) {
            *slot = width;
        }
        self.settings
            .column_widths
            .insert(table.key().to_string(), widths);
    }

    /// 包住一个表头单元格：设置列宽，并在右缘放置拖拽柄（双击恢复默认宽度）。
    pub(super) fn header_cell(
        &self,
        table: TableId,
        column: usize,
        width: f32,
        content: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let dragging = self
            .column_drag
            .is_some_and(|drag| drag.table == table && drag.column == column);
        let group: SharedString = format!("col-{}-{column}", table.key()).into();
        div()
            .relative()
            .flex()
            .items_center()
            .h_full()
            .w(px(width))
            .flex_shrink_0()
            .pr_2()
            .child(content)
            .child(
                div()
                    .id(SharedString::from(format!(
                        "col-handle-{}-{column}",
                        table.key()
                    )))
                    .group(group.clone())
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-4.))
                    .w(px(8.))
                    .flex()
                    .justify_center()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if event.click_count >= 2 {
                                let default = table.defaults()[column];
                                this.set_column_width(table, column, default);
                                this.column_drag = None;
                                this.save_settings();
                            } else {
                                this.column_drag = Some(ColumnDrag {
                                    table,
                                    column,
                                    start_x: event.position.x,
                                    start_width: this.column_widths(table)[column],
                                });
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(2.))
                            .h(px(16.))
                            .my_auto()
                            .rounded(px(1.))
                            .bg(if dragging {
                                theme::accent()
                            } else {
                                theme::border_strong()
                            })
                            .opacity(if dragging { 1. } else { 0. })
                            .group_hover(group, |style| style.opacity(1.)),
                    ),
            )
    }

    /// 根元素的鼠标移动：拖拽中时更新列宽。松开按钮（可能发生在窗口外）即结束拖拽。
    pub(super) fn on_column_drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.column_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_column_drag(cx);
            return;
        }
        let delta: f32 = (event.position.x - drag.start_x).into();
        let width = dragged_width(drag.start_width, delta, theme::ui_scale());
        self.set_column_width(drag.table, drag.column, width);
        cx.notify();
    }

    pub(super) fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if self.column_drag.take().is_some() {
            self.save_settings();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WIDTH, MIN_WIDTH, TableId, dragged_width};

    #[test]
    fn dragged_width_respects_scale_and_bounds() {
        assert_eq!(dragged_width(100., 50., 1.0), 150.);
        assert_eq!(dragged_width(100., 50., 1.25), 140.);
        assert_eq!(dragged_width(100., -500., 1.0), MIN_WIDTH);
        assert_eq!(dragged_width(100., 900., 1.0), MAX_WIDTH);
    }

    #[test]
    fn defaults_exclude_the_flexible_last_column() {
        assert_eq!(TableId::Scan.defaults().len(), 5);
        assert_eq!(TableId::Speed.defaults().len(), 6);
        assert_eq!(TableId::Verify.defaults().len(), 5);
    }
}
