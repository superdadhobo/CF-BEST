use std::ops::Range;

use gpui::{ClickEvent, Context, FontWeight, Window, div, prelude::*};

use crate::theme::px;

use super::AppState;
use super::columns::TableId;
use crate::core::model::{ParamGroup, SpeedRecord};
use crate::jobs::JobKind;
use crate::theme;
use crate::ui::chart;
use crate::ui::widgets::{self, ButtonKind, Tone};

/// 测速表格的排序列。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedSortKey {
    Median,
    Best,
    Min,
    Colo,
}

impl SpeedSortKey {
    /// 首次点击某列时的默认方向：数值列降序，落点按字母升序。
    fn default_desc(self) -> bool {
        !matches!(self, SpeedSortKey::Colo)
    }
}

fn speed_order(records: &[SpeedRecord], key: SpeedSortKey, desc: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&records[a], &records[b]);
        let ordering = match key {
            SpeedSortKey::Median => a.median.total_cmp(&b.median),
            SpeedSortKey::Best => a.best.total_cmp(&b.best),
            SpeedSortKey::Min => a.min.total_cmp(&b.min),
            SpeedSortKey::Colo => a.colo.cmp(&b.colo).then(b.median.total_cmp(&a.median)),
        };
        if desc { ordering.reverse() } else { ordering }
    });
    order
}

impl AppState {
    fn set_speed_sort(&mut self, key: SpeedSortKey, cx: &mut Context<Self>) {
        self.speed_sort = if self.speed_sort.0 == key {
            (key, !self.speed_sort.1)
        } else {
            (key, key.default_desc())
        };
        cx.notify();
    }

    fn speed_sort_header(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (current, desc) = self.speed_sort;
        let widths = self.column_widths(TableId::Speed);
        let sortable = |name: &'static str, key: SpeedSortKey, cx: &mut Context<Self>| {
            widgets::sortable_header_cell(name, (current == key).then_some(desc))
                .id(name)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_speed_sort(key, cx);
                }))
        };
        let colo = sortable("机房", SpeedSortKey::Colo, cx);
        let median = sortable("中位", SpeedSortKey::Median, cx);
        let best = sortable("最快", SpeedSortKey::Best, cx);
        let min = sortable("最慢", SpeedSortKey::Min, cx);
        widgets::table_header_row()
            .child(self.header_cell(
                TableId::Speed,
                0,
                widths[0],
                widgets::plain_header_cell("IP"),
                cx,
            ))
            .child(self.header_cell(TableId::Speed, 1, widths[1], colo, cx))
            .child(self.header_cell(TableId::Speed, 2, widths[2], median, cx))
            .child(self.header_cell(TableId::Speed, 3, widths[3], best, cx))
            .child(self.header_cell(TableId::Speed, 4, widths[4], min, cx))
            .child(self.header_cell(
                TableId::Speed,
                5,
                widths[5],
                widgets::plain_header_cell("稳定"),
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .child(widgets::plain_header_cell("各轮速度（Mbps）")),
            )
    }

    fn copy_speed_ips(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ips = self
            .speed_view_order
            .iter()
            .filter_map(|&index| self.speed_records.get(index))
            .map(|record| record.ip.clone())
            .collect::<Vec<_>>();
        let count = ips.len();
        self.copy_text(ips.join("\n"), &format!(" {count} 个 IP"), cx);
    }

    /// 测速分布：中位吞吐直方图 + 延迟 / 吞吐散点（表格即其表格视图）。
    fn render_speed_charts(&self) -> Option<gpui::Div> {
        if self.speed_records.len() < 3 {
            return None;
        }
        let medians: Vec<f64> = self
            .speed_records
            .iter()
            .map(|record| record.median)
            .collect();
        let bins = crate::core::stats::histogram(&medians, 12);
        let step = bins.first().map(|bin| bin.end - bin.start).unwrap_or(0.0);
        let latency: std::collections::HashMap<&str, f64> = self
            .scan_hits
            .iter()
            .map(|hit| (hit.ip.as_str(), hit.latency_ms))
            .collect();
        let points: Vec<chart::ScatterPoint> = self
            .speed_records
            .iter()
            .filter_map(|record| {
                latency
                    .get(record.ip.as_str())
                    .map(|ms| chart::ScatterPoint {
                        x: *ms,
                        y: record.median,
                        title: format!("{} · {}", record.ip, record.colo).into(),
                    })
            })
            .collect();
        let best = points
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))
            .map(|(index, _)| index);
        let chart_title = |title: &'static str, detail: String| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .mb_2()
                .child(
                    div()
                        .text_size(px(theme::font::LABEL))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(theme::font::CAPTION))
                        .text_color(theme::faint())
                        .child(detail),
                )
        };
        let scatter = if points.len() >= 3 {
            div()
                .flex()
                .flex_col()
                .child(chart_title(
                    "握手延迟与下载速度",
                    format!(
                        "{} 个 IP 有握手延迟数据 · 横轴为延迟（ms），纵轴为速度（Mbps）",
                        points.len()
                    ),
                ))
                .child(chart::scatter(
                    "speed-scatter",
                    &points,
                    "ms",
                    "Mbps",
                    best,
                    200.,
                ))
        } else {
            div()
                .flex()
                .flex_col()
                .child(chart_title(
                    "握手延迟与下载速度",
                    "需要扫描结果中的握手延迟".into(),
                ))
                .child(
                    widgets::empty_state(
                        "activity",
                        "延迟数据不足",
                        "从扫描结果载入测速候选后，这里会显示延迟与速度的关系",
                    )
                    .h(px(200.)),
                )
        };
        Some(
            widgets::card()
                .child(widgets::section_title("trending-up", "测速分布"))
                .child(
                    div()
                        .flex()
                        .gap_6()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(chart_title(
                                    "中位速度分布",
                                    format!(
                                        "{} 个 IP · 组距 {} Mbps",
                                        medians.len(),
                                        if step.fract().abs() < 1e-9 {
                                            format!("{step:.0}")
                                        } else {
                                            format!("{step:.1}")
                                        }
                                    ),
                                ))
                                .child(chart::histogram("speed-hist", &bins, "Mbps", 200.)),
                        )
                        .child(div().flex_1().min_w_0().child(scatter)),
                ),
        )
    }

    pub fn render_speed_page(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let advanced = self.settings.advanced_mode;
        let running = self.job == Some(JobKind::Speed);
        let busy = self.busy();
        let progress = self.speed_progress.clone();
        let records = self.speed_records.len();
        let scan_hits = self.scan_hits.len();
        let best = self
            .speed_records
            .iter()
            .map(|record| record.median)
            .fold(0.0, f64::max);
        let status = if running {
            ("测速中", Tone::Accent)
        } else if records > 0 {
            ("已完成", Tone::Green)
        } else {
            ("就绪", Tone::Muted)
        };

        let advanced_fields = advanced.then(|| {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_3()
                .rounded(px(theme::radius::LG))
                .bg(theme::panel_alt())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::icon("sliders", 14.0, theme::faint()))
                        .child(
                            div()
                                .text_size(px(theme::font::LABEL))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::muted())
                                .child("测速参数"),
                        )
                        .child(div().flex_1())
                        .children(self.restore_params_button(ParamGroup::Speed, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .child(widgets::field("初筛时长（秒）", self.speed_coarse.clone()))
                        .child(widgets::field("精测时长（秒）", self.speed_fine.clone()))
                        .child(widgets::field("精测轮数", self.speed_rounds.clone()))
                        .child(widgets::field("精测数量", self.speed_topn.clone())),
                )
                .child(widgets::hint(
                    "逐个串行测速：候选越多，耗时越长；精测数量越多，排名越完整。",
                ))
        });

        let action_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                widgets::button_with_state(
                    "start-speed",
                    "开始测速",
                    "play",
                    ButtonKind::Primary,
                    busy,
                )
                .on_click(cx.listener(Self::start_speed)),
            )
            .when(scan_hits > 0, |row| {
                row.child(
                    widgets::button(
                        "load-scan-hits",
                        format!("载入扫描结果 ({scan_hits})"),
                        "folder-open",
                        ButtonKind::Secondary,
                    )
                    .on_click(cx.listener(Self::load_scan_hits_to_speed)),
                )
            })
            .when(advanced, |row| {
                row.child(
                    widgets::button_with_state(
                        "pool-quick",
                        "官方候选 120",
                        "database",
                        ButtonKind::Ghost,
                        busy,
                    )
                    .on_click(cx.listener(Self::pool_quick)),
                )
                .child(
                    widgets::button_with_state(
                        "pool-full",
                        "官方候选 400",
                        "database",
                        ButtonKind::Ghost,
                        busy,
                    )
                    .on_click(cx.listener(Self::pool_full)),
                )
            })
            .when(!advanced, |row| {
                row.child(
                    widgets::button_with_state(
                        "pool-quick",
                        "官方候选池",
                        "database",
                        ButtonKind::Ghost,
                        busy,
                    )
                    .on_click(cx.listener(Self::pool_quick)),
                )
            });

        let progress_text = if !self.speed_current.is_empty() {
            self.speed_current.clone()
        } else if !progress.label.is_empty() {
            progress.label.clone()
        } else {
            "尚未开始测速".to_string()
        };

        let target_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("gauge", "测速候选"))
                    .child(widgets::pill(status.0, status.1)),
            )
            .child(widgets::hint(if advanced {
                "每行一个 IP 或 IP:端口；也可以用官方候选池生成。"
            } else {
                "从扫描结果自动带入，也可以手动粘贴 IP。"
            }))
            .child(self.speed_input.clone())
            .children(advanced_fields)
            .child(action_row)
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
                            .child(widgets::progress_caption(
                                progress_text,
                                running,
                                "speed-spinner",
                            ))
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(px(theme::font::SMALL))
                                    .text_color(theme::faint())
                                    .child(format!("{:.0}%", progress.fraction() * 100.0)),
                            ),
                    )
                    .child(widgets::progress_bar(
                        "speed-progress",
                        progress.fraction(),
                        running,
                    )),
            )
            .when(advanced, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_5()
                        .pt_3()
                        .border_t_1()
                        .border_color(theme::border())
                        .child(widgets::stat(
                            "阶段",
                            if self.speed_phase.is_empty() {
                                "空闲".to_string()
                            } else {
                                self.speed_phase.clone()
                            },
                            if running { Tone::Accent } else { Tone::Muted },
                        ))
                        .child(widgets::stat("有效", records.to_string(), Tone::Green))
                        .child(widgets::stat(
                            "429",
                            self.speed_throttled.to_string(),
                            if self.speed_throttled > 0 {
                                Tone::Amber
                            } else {
                                Tone::Muted
                            },
                        ))
                        .child(div().flex_1())
                        .child(widgets::hint(if best > 0.0 {
                            format!("当前最快中位速度：{best:.1} Mbps")
                        } else if running {
                            "正在逐个测试候选，请稍候。".to_string()
                        } else {
                            "还没有有效的测速结果。".to_string()
                        })),
                )
            })
            .when(!advanced && records == 0 && !running, |this| {
                this.child(widgets::inline_guide([
                    "准备候选",
                    "逐个测速",
                    "点「下一步：验证」",
                ]))
            });

        let warnings = if self.speed_warnings.is_empty() {
            None
        } else {
            Some(
                widgets::card()
                    .bg(theme::amber_dim())
                    .border_color(theme::amber_dim())
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::icon("alert-triangle", 15.0, theme::amber()))
                            .child(
                                div()
                                    .text_size(px(theme::font::BODY_LG))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::amber())
                                    .child("限流提示"),
                            ),
                    )
                    .children(self.speed_warnings.iter().take(3).map(|warning| {
                        div()
                            .text_size(px(theme::font::LABEL))
                            .line_height(px(20.))
                            .text_color(theme::amber())
                            .child(warning.clone())
                    })),
            )
        };

        self.speed_view_order =
            speed_order(&self.speed_records, self.speed_sort.0, self.speed_sort.1);
        let sort_hint = match self.speed_sort {
            (SpeedSortKey::Median, true) => "按中位速度从高到低排序 · 单位 Mbps · 点击行复制 IP",
            _ => "点击表头切换排序 · 点击行复制 IP",
        };
        let row_count = self.speed_records.len();
        let table_body = if row_count == 0 {
            widgets::empty_state(
                "trending-up",
                if running {
                    "测速进行中…"
                } else {
                    "还没有测速结果"
                },
                if running {
                    "精测结果会逐个出现在这里"
                } else {
                    "从扫描结果载入，或使用官方候选池"
                },
            )
            .h(px(160.))
            .into_any_element()
        } else {
            gpui::uniform_list(
                "speed-rows",
                row_count,
                cx.processor(|this: &mut AppState, range: Range<usize>, _window, cx| {
                    let widths = this.column_widths(TableId::Speed);
                    let mut rows = Vec::with_capacity(range.end - range.start);
                    for ix in range {
                        let Some(record) = this
                            .speed_view_order
                            .get(ix)
                            .and_then(|&index| this.speed_records.get(index))
                        else {
                            continue;
                        };
                        let ip = record.ip.clone();
                        let stable = record.is_stable();
                        let rounds = record
                            .rounds
                            .iter()
                            .map(|value| format!("{value:.1}"))
                            .collect::<Vec<_>>()
                            .join(" / ");
                        rows.push(
                            div()
                                .id(ix)
                                .flex()
                                .w_full()
                                .items_center()
                                .h(px(42.))
                                .px_4()
                                .bg(if ix % 2 == 0 {
                                    theme::row_even()
                                } else {
                                    theme::panel()
                                })
                                .border_b_1()
                                .border_color(theme::border())
                                .hover(|style| style.bg(theme::row_hover()))
                                .group("speed-row")
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.copy_text(ip.clone(), &format!(" {ip}"), cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .w(px(widths[0]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY_LG))
                                        .child(record.ip.clone())
                                        .child(widgets::copy_hint("speed-row")),
                                )
                                .child(
                                    div()
                                        .w(px(widths[1]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY))
                                        .text_color(theme::green())
                                        .child(record.colo.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(widths[2]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY_LG))
                                        .text_color(theme::text())
                                        .child(format!("{:.1}", record.median)),
                                )
                                .child(
                                    div()
                                        .w(px(widths[3]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY))
                                        .text_color(theme::muted())
                                        .child(format!("{:.1}", record.best)),
                                )
                                .child(
                                    div()
                                        .w(px(widths[4]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY))
                                        .text_color(theme::muted())
                                        .child(format!("{:.1}", record.min)),
                                )
                                .child(div().w(px(widths[5])).flex_shrink_0().child(widgets::pill(
                                    if stable { "稳定" } else { "波动" },
                                    if stable { Tone::Green } else { Tone::Amber },
                                )))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::SMALL))
                                        .text_color(theme::muted())
                                        .child(rounds),
                                ),
                        );
                    }
                    rows
                }),
            )
            .h(px(330.))
            .w_full()
            .into_any_element()
        };

        let table_actions = (records > 0).then(|| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    widgets::button(
                        "speed-to-verify",
                        "下一步：验证",
                        "shield-check",
                        ButtonKind::Soft,
                    )
                    .on_click(cx.listener(Self::next_to_verify)),
                )
                .child(
                    widgets::button("copy-speed-ips", "复制全部", "copy", ButtonKind::Ghost)
                        .on_click(cx.listener(Self::copy_speed_ips)),
                )
                .child(
                    widgets::button("export-speed", "导出", "download", ButtonKind::Ghost)
                        .on_click(cx.listener(Self::export_speed)),
                )
        });

        let table_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("activity", "测速排行"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::pill_action(
                                format!("{records} 个"),
                                if records > 0 {
                                    Tone::Green
                                } else {
                                    Tone::Muted
                                },
                            ))
                            .child(
                                div()
                                    .text_size(px(theme::font::SMALL))
                                    .text_color(theme::faint())
                                    .child(sort_hint),
                            )
                            .children(table_actions),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .rounded(px(theme::radius::LG))
                    .overflow_hidden()
                    .child(self.speed_sort_header(cx))
                    .child(table_body),
            );

        div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(target_card)
            .children(warnings)
            .children(self.render_speed_charts())
            .child(table_card)
    }
}

#[cfg(test)]
mod tests {
    use super::{SpeedSortKey, speed_order};
    use crate::core::model::SpeedRecord;

    fn record(ip: &str, colo: &str, median: f64, best: f64, min: f64) -> SpeedRecord {
        SpeedRecord {
            ip: ip.into(),
            port: 443,
            colo: colo.into(),
            median,
            best,
            min,
            rounds: vec![],
        }
    }

    #[test]
    fn speed_order_sorts_by_key_and_direction() {
        let records = vec![
            record("a", "SJC", 10.0, 30.0, 5.0),
            record("b", "HKG", 30.0, 35.0, 1.0),
            record("c", "LAX", 20.0, 40.0, 9.0),
        ];
        assert_eq!(speed_order(&records, SpeedSortKey::Median, true), [1, 2, 0]);
        assert_eq!(
            speed_order(&records, SpeedSortKey::Median, false),
            [0, 2, 1]
        );
        assert_eq!(speed_order(&records, SpeedSortKey::Best, true), [2, 1, 0]);
        assert_eq!(speed_order(&records, SpeedSortKey::Min, true), [2, 0, 1]);
        assert_eq!(speed_order(&records, SpeedSortKey::Colo, false), [1, 2, 0]);
    }
}
