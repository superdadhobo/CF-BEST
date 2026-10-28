//! 轻量图表（纯 div 绘制）：直方图与散点图。
//!
//! 规格来自 Qoder 图表（12/16px 文字、200 / 240px 绘图区、Tooltip ≤ 280px 圆角 8）与 dataviz 方法：
//! 单系列不画图例（标题即说明）；柱宽 ≤ 24px、顶端 4px 圆角、相邻柱间 2px 留白；
//! 散点 8px 圆点 + 2px 表面色描边、24px 命中区；网格为实线细线；只直接标注极值；
//! 文字一律使用文字色，不使用系列色。每个图表都另有表格可读到全部数值。

use gpui::{
    AnyView, App, Context, Div, IntoElement, Render, Rgba, SharedString, Window, div, prelude::*,
    relative,
};

use crate::core::stats::{self, Bin};
use crate::theme::{self, px};

const AXIS_WIDTH: f32 = 40.0;

/// Tooltip 中的一行：系列色短线 + 数值（强调）+ 名称（次要）。
pub struct ChartTooltip {
    title: SharedString,
    rows: Vec<(Rgba, SharedString, SharedString)>,
}

impl Render for ChartTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .max_w(px(280.))
            .px_3()
            .py_2()
            .rounded(px(theme::radius::MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow(theme::shadow::raised())
            .font_family(theme::FONT_UI)
            .child(
                div()
                    .mb_2()
                    .text_size(px(theme::font::MICRO))
                    .line_height(px(14.))
                    .text_color(theme::faint())
                    .child(self.title.clone()),
            )
            .children(self.rows.iter().map(|(color, value, label)| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(10.)).h(px(3.)).rounded(px(2.)).bg(*color))
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px(theme::font::BODY_LG))
                            .text_color(theme::text())
                            .child(value.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::font::CAPTION))
                            .text_color(theme::muted())
                            .child(label.clone()),
                    )
            }))
    }
}

fn chart_tooltip(
    title: SharedString,
    rows: Vec<(Rgba, SharedString, SharedString)>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        let title = title.clone();
        let rows = rows.clone();
        cx.new(|_| ChartTooltip { title, rows }).into()
    }
}

fn format_number(value: f64) -> String {
    if value >= 100.0 || value.fract().abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn axis_label(text: String) -> Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(theme::font::MICRO))
        .line_height(px(14.))
        .text_color(theme::faint())
        .child(text)
}

/// 纵轴刻度 + 横向网格线（从底部按比例摆放）。
fn y_axis(ticks: &[f64], ceiling: f64, height: f32) -> (Div, Vec<Div>) {
    let mut labels = div()
        .relative()
        .w(px(AXIS_WIDTH))
        .h(px(height))
        .flex_shrink_0();
    let mut grid = Vec::new();
    for tick in ticks {
        let fraction = (tick / ceiling) as f32;
        labels = labels.child(
            div()
                .absolute()
                .right(px(8.))
                .bottom(relative(fraction))
                .mb(px(-7.))
                .child(axis_label(format_number(*tick))),
        );
        if *tick > 0.0 {
            grid.push(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(relative(fraction))
                    .h(px(1.))
                    .bg(theme::border_subtle()),
            );
        }
    }
    (labels, grid)
}

/// 直方图：`unit` 用于 Tooltip 与横轴说明（如 `Mbps`）。
pub fn histogram(id: &'static str, bins: &[Bin], unit: &'static str, height: f32) -> Div {
    let color = theme::chart_series();
    let max_count = bins.iter().map(|bin| bin.count).max().unwrap_or(0).max(1);
    let (ceiling, step) = stats::nice_ceiling(max_count as f64, 4);
    let step = step.max(1.0);
    let ceiling = ceiling.max(step);
    let ticks = stats::axis_ticks(ceiling, step);
    let (labels, grid) = y_axis(&ticks, ceiling, height);
    let peak = bins
        .iter()
        .enumerate()
        .max_by_key(|(_, bin)| bin.count)
        .map(|(index, _)| index);
    let label_every = bins.len().div_ceil(8).max(1);

    let columns = bins.iter().enumerate().map(|(index, bin)| {
        let group: SharedString = format!("{id}-bin-{index}").into();
        let fraction = (bin.count as f64 / ceiling) as f32;
        let range: SharedString = format!(
            "{}–{} {unit}",
            format_number(bin.start),
            format_number(bin.end)
        )
        .into();
        div()
            .id((id, index))
            .group(group.clone())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .px(px(1.))
            .tooltip(chart_tooltip(
                range,
                vec![(color, format!("{}", bin.count).into(), "个 IP".into())],
            ))
            .children((Some(index) == peak && bin.count > 0).then(|| {
                div()
                    .mb_1()
                    .child(axis_label(bin.count.to_string()).text_color(theme::muted()))
            }))
            .child(
                div()
                    .w_full()
                    .max_w(px(24.))
                    .h(relative(fraction))
                    .rounded_t(px(4.))
                    .bg(color)
                    .group_hover(group, |style| style.opacity(0.78)),
            )
    });

    // 横轴刻度落在分箱边界上（每 `label_every` 个边界一个，另加最右侧边界）。
    let edge_count = bins.len();
    let x_labels = (0..=edge_count)
        .filter(|edge| edge % label_every == 0 || *edge == edge_count)
        .map(|edge| {
            let value = bins
                .get(edge)
                .map(|bin| bin.start)
                .or_else(|| bins.last().map(|bin| bin.end))
                .unwrap_or_default();
            div()
                .absolute()
                .left(relative(edge as f32 / edge_count.max(1) as f32))
                .ml(px(-20.))
                .w(px(40.))
                .flex()
                .justify_center()
                .child(axis_label(format_number(value)))
        });

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div().flex().child(labels).child(
                div()
                    .relative()
                    .flex_1()
                    .h(px(height))
                    .border_b_1()
                    .border_color(theme::border_strong())
                    .children(grid)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_end()
                            .children(columns),
                    ),
            ),
        )
        .child(
            div()
                .flex()
                .child(div().w(px(AXIS_WIDTH)).flex_shrink_0())
                .child(div().relative().flex_1().h(px(14.)).children(x_labels)),
        )
}

/// 散点图中的一个点。
#[derive(Clone, Debug)]
pub struct ScatterPoint {
    pub x: f64,
    pub y: f64,
    pub title: SharedString,
}

/// 散点图：`highlight` 为直接标注的点（极值）。
pub fn scatter(
    id: &'static str,
    points: &[ScatterPoint],
    x_unit: &'static str,
    y_unit: &'static str,
    highlight: Option<usize>,
    height: f32,
) -> Div {
    let color = theme::chart_series();
    let max_x = points.iter().map(|point| point.x).fold(0.0, f64::max);
    let max_y = points.iter().map(|point| point.y).fold(0.0, f64::max);
    let (x_ceiling, x_step) = stats::nice_ceiling(max_x.max(1.0), 5);
    let (y_ceiling, y_step) = stats::nice_ceiling(max_y.max(1.0), 4);
    let (labels, mut grid) = y_axis(&stats::axis_ticks(y_ceiling, y_step), y_ceiling, height);
    let x_ticks = stats::axis_ticks(x_ceiling, x_step);
    for tick in &x_ticks {
        if *tick > 0.0 {
            grid.push(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative((tick / x_ceiling) as f32))
                    .w(px(1.))
                    .bg(theme::border_subtle()),
            );
        }
    }

    let dots = points.iter().enumerate().map(|(index, point)| {
        let group: SharedString = format!("{id}-pt-{index}").into();
        let left = (point.x / x_ceiling) as f32;
        let top = 1.0 - (point.y / y_ceiling) as f32;
        div()
            .id((id, index))
            .group(group.clone())
            .absolute()
            .left(relative(left))
            .top(relative(top))
            .ml(px(-12.))
            .mt(px(-12.))
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .tooltip(chart_tooltip(
                point.title.clone(),
                vec![
                    (color, format!("{:.1}", point.y).into(), y_unit.into()),
                    (color, format!("{:.0}", point.x).into(), x_unit.into()),
                ],
            ))
            .child(
                div()
                    .size(px(12.))
                    .rounded(px(6.))
                    .border_2()
                    .border_color(theme::content())
                    .bg(color)
                    .group_hover(group, |style| style.size(px(14.))),
            )
            .children((Some(index) == highlight).then(|| {
                // 靠右的点把标注放到左侧，避免超出图表被裁掉。
                let label = div()
                    .absolute()
                    .top(px(4.))
                    .whitespace_nowrap()
                    .child(axis_label(point.title.to_string()).text_color(theme::muted()));
                if left > 0.6 {
                    label.right(px(22.))
                } else {
                    label.left(px(22.))
                }
            }))
    });

    let x_labels = x_ticks.iter().map(|tick| {
        div()
            .absolute()
            .left(relative((tick / x_ceiling) as f32))
            .ml(px(-20.))
            .w(px(40.))
            .flex()
            .justify_center()
            .child(axis_label(format_number(*tick)))
    });

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div().flex().child(labels).child(
                div()
                    .relative()
                    .flex_1()
                    .h(px(height))
                    .border_b_1()
                    .border_l_1()
                    .border_color(theme::border_strong())
                    .children(grid)
                    .children(dots),
            ),
        )
        .child(
            div()
                .flex()
                .child(div().w(px(AXIS_WIDTH)).flex_shrink_0())
                .child(div().relative().flex_1().h(px(14.)).children(x_labels)),
        )
}
