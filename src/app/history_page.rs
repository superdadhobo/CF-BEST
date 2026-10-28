use gpui::{Context, div, prelude::*};

use crate::theme::px;

use super::AppState;
use crate::theme;
use crate::ui::widgets::{self, ButtonKind, Tone};

/// `2026-10-20 19:36:32` → `10-20 19:36`。
fn short_time(time: &str) -> &str {
    time.get(5..16).unwrap_or(time)
}

impl AppState {
    /// 两次测速的 Top 10 对比：排名变化与中位吞吐差值。超过 7 项，用表格而不是图。
    fn render_speed_compare(&self, cx: &mut Context<Self>) -> gpui::Div {
        let runs = &self.history.speed;
        let card = widgets::card().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(widgets::section_title("trending-up", "测速对比"))
                .child(widgets::hint("最近一次的 Top 10 与较早一次对比")),
        );
        if runs.len() < 2 {
            return card.child(
                widgets::empty_state(
                    "trending-up",
                    "至少需要两次测速",
                    "再完成一次测速后，这里会显示排名与速度的变化",
                )
                .h(px(120.)),
            );
        }
        let base_index = self.history_compare_base.clamp(1, runs.len() - 1);
        let current = &runs[0];
        let base = &runs[base_index];
        let (rows, dropped) = crate::core::stats::compare_runs(&current.results, &base.results, 10);

        let selector = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(theme::radius::XL))
            .bg(theme::fill_secondary())
            .children(runs.iter().enumerate().skip(1).take(5).map(|(index, run)| {
                let active = index == base_index;
                div()
                    .id(("compare-base", index))
                    .px_2p5()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .rounded(px(theme::radius::MD))
                    .cursor_pointer()
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
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        this.history_compare_base = index;
                        cx.notify();
                    }))
                    .child(short_time(&run.time).to_string())
            }));

        let header = widgets::table_header_row()
            .child(
                div()
                    .w(px(56.))
                    .flex_shrink_0()
                    .child(widgets::plain_header_cell("排名")),
            )
            .child(
                div()
                    .w(px(200.))
                    .flex_shrink_0()
                    .child(widgets::plain_header_cell("IP")),
            )
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(widgets::plain_header_cell("机房")),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_shrink_0()
                    .child(widgets::plain_header_cell("中位速度")),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .child(widgets::plain_header_cell("速度变化")),
            )
            .child(div().flex_1().child(widgets::plain_header_cell("排名变化")));

        let body = rows.iter().enumerate().map(|(index, row)| {
            let speed_change = match row.median_change() {
                Some(delta) if delta.abs() < 0.05 => div().text_color(theme::faint()).child("±0.0"),
                Some(delta) => div()
                    .text_color(if delta > 0.0 {
                        theme::green()
                    } else {
                        theme::red()
                    })
                    .child(format!(
                        "{} {:+.1}",
                        if delta > 0.0 { "▲" } else { "▼" },
                        delta
                    )),
                None => div().text_color(theme::faint()).child("—"),
            };
            let rank_change = match row.rank_change() {
                None => widgets::pill("新上榜", Tone::Blue),
                Some(0) => widgets::pill("持平", Tone::Muted),
                Some(change) if change > 0 => widgets::pill(
                    format!("▲ {change}（原 #{}）", row.prev_rank.unwrap_or_default()),
                    Tone::Green,
                ),
                Some(change) => widgets::pill(
                    format!("▼ {}（原 #{}）", -change, row.prev_rank.unwrap_or_default()),
                    Tone::Red,
                ),
            };
            div()
                .flex()
                .items_center()
                .h(px(40.))
                .px_4()
                .bg(if index % 2 == 0 {
                    theme::row_even()
                } else {
                    theme::panel()
                })
                .border_b_1()
                .border_color(theme::border_subtle())
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::font::BODY))
                .child(
                    div()
                        .w(px(56.))
                        .flex_shrink_0()
                        .text_color(theme::muted())
                        .child(format!("#{}", row.rank)),
                )
                .child(
                    div()
                        .w(px(200.))
                        .flex_shrink_0()
                        .text_color(theme::text())
                        .child(row.ip.clone()),
                )
                .child(
                    div()
                        .w(px(80.))
                        .flex_shrink_0()
                        .text_color(theme::muted())
                        .child(row.colo.clone()),
                )
                .child(
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .text_color(theme::text())
                        .child(format!("{:.1}", row.median)),
                )
                .child(div().w(px(120.)).flex_shrink_0().child(speed_change))
                .child(div().flex_1().flex().child(rank_change))
        });

        card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(px(theme::font::SMALL))
                        .text_color(theme::muted())
                        .child(format!("当前 {} · 对比基准", short_time(&current.time))),
                )
                .child(selector),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(theme::radius::LG))
                .overflow_hidden()
                .child(header)
                .children(body),
        )
        .child(widgets::hint(if dropped > 0 {
            format!("基准 Top 10 中有 {dropped} 个 IP 这次未进入 Top 10。")
        } else {
            "基准 Top 10 的 IP 这次都仍在 Top 10 内。".to_string()
        }))
    }

    pub fn render_history_page(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let daily_runs = self
            .history
            .daily
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let verify_runs = self
            .history
            .verify
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let speed_runs = self
            .history
            .speed
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>();

        let header = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(widgets::section_title("database", "本地记录"))
                            .child(widgets::hint(
                                "所有记录保存在应用数据目录，最多保留 60 次运行。",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                widgets::button(
                                    "refresh-history",
                                    "刷新",
                                    "refresh",
                                    ButtonKind::Secondary,
                                )
                                .on_click(cx.listener(Self::refresh_history)),
                            )
                            .child(
                                widgets::button(
                                    "history-open-data",
                                    "打开数据目录",
                                    "folder-open",
                                    ButtonKind::Ghost,
                                )
                                .on_click(cx.listener(Self::open_data_dir)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(widgets::stat(
                        "每日优选",
                        format!("{} 次", self.history.daily.len()),
                        Tone::Accent,
                    ))
                    .child(widgets::stat(
                        "可用性验证",
                        format!("{} 次", self.history.verify.len()),
                        Tone::Green,
                    ))
                    .child(widgets::stat(
                        "测速",
                        format!("{} 次", self.history.speed.len()),
                        Tone::Blue,
                    )),
            );

        let daily_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("calendar", "每日优选"))
                    .child(widgets::hint("最近 8 次")),
            )
            .children(if daily_runs.is_empty() {
                vec![
                    widgets::empty_state("archive", "暂无记录", "运行一次每日优选后会出现在这里")
                        .h(px(110.)),
                ]
            } else {
                daily_runs
                    .iter()
                    .map(|run| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_3()
                            .rounded(px(theme::radius::MD))
                            .bg(theme::panel_alt())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_family(theme::FONT_MONO)
                                            .text_size(px(theme::font::BODY))
                                            .text_color(theme::muted())
                                            .child(run.time.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .when_some(
                                                run.top
                                                    .first()
                                                    .and_then(|entry| entry.verification.known()),
                                                |row, verification| {
                                                    row.child(widgets::pill(
                                                        verification.label(),
                                                        Tone::Muted,
                                                    ))
                                                },
                                            )
                                            .child(widgets::pill(
                                                format!("Top {}", run.top.len()),
                                                if run.top.is_empty() {
                                                    Tone::Muted
                                                } else {
                                                    Tone::Green
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .text_size(px(theme::font::SMALL))
                                                    .text_color(theme::faint())
                                                    .child(format!(
                                                        "扫描 {} · 命中 {} · 测速 {}",
                                                        run.scanned, run.hits, run.tested
                                                    )),
                                            ),
                                    ),
                            )
                            .children(if run.top.is_empty() {
                                vec![
                                    div()
                                        .text_size(px(theme::font::LABEL))
                                        .text_color(theme::faint())
                                        .child("本轮无可用 IP")
                                        .into_any_element(),
                                ]
                            } else {
                                run.top
                                    .iter()
                                    .map(|entry| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_MONO)
                                                    .text_size(px(theme::font::BODY))
                                                    .text_color(theme::text())
                                                    .child(format!("{}:{}", entry.ip, entry.port)),
                                            )
                                            .child(widgets::pill(entry.colo.clone(), Tone::Green))
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_MONO)
                                                    .text_size(px(theme::font::SMALL))
                                                    .text_color(theme::text())
                                                    .child(format!("{:.1} Mbps", entry.best_mbps)),
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            })
                    })
                    .collect()
            });

        let verify_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("shield-check", "可用性验证"))
                    .child(widgets::hint("最近 8 次")),
            )
            .children(if verify_runs.is_empty() {
                vec![
                    widgets::empty_state(
                        "badge-check",
                        "暂无记录",
                        "完成一次可用性验证后会出现在这里",
                    )
                    .h(px(110.)),
                ]
            } else {
                verify_runs
                    .iter()
                    .map(|run| {
                        let passed: Vec<String> = run
                            .records
                            .iter()
                            .filter(|record| record.ok)
                            .take(6)
                            .map(|record| {
                                format!(
                                    "{}#{}",
                                    record.ip,
                                    record.colo.clone().unwrap_or_else(|| "-".into())
                                )
                            })
                            .collect();
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_3()
                            .rounded(px(theme::radius::MD))
                            .bg(theme::panel_alt())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_family(theme::FONT_MONO)
                                            .text_size(px(theme::font::BODY))
                                            .text_color(theme::muted())
                                            .child(run.time.clone()),
                                    )
                                    .child(widgets::pill(
                                        format!("{} / {} 通过", run.passed, run.total),
                                        if run.passed > 0 {
                                            Tone::Green
                                        } else {
                                            Tone::Red
                                        },
                                    )),
                            )
                            .child(div().flex().flex_wrap().gap_2().children(passed.iter().map(
                                |item| {
                                    div()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::SMALL))
                                        .text_color(theme::muted())
                                        .child(item.clone())
                                },
                            )))
                    })
                    .collect()
            });

        let speed_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("gauge", "测速"))
                    .child(widgets::hint("最近 5 次")),
            )
            .children(if speed_runs.is_empty() {
                vec![
                    widgets::empty_state("clock", "暂无记录", "完成一次测速后会出现在这里")
                        .h(px(110.)),
                ]
            } else {
                speed_runs
                    .iter()
                    .map(|run| {
                        let best = run.results.first();
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .p_3()
                            .rounded(px(theme::radius::MD))
                            .bg(theme::panel_alt())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .font_family(theme::FONT_MONO)
                                            .text_size(px(theme::font::BODY))
                                            .text_color(theme::muted())
                                            .child(run.time.clone()),
                                    )
                                    .child(widgets::pill(
                                        format!("{} 个结果", run.results.len()),
                                        Tone::Blue,
                                    ))
                                    .children((run.throttled > 0).then(|| {
                                        widgets::pill(
                                            format!("429 ×{}", run.throttled),
                                            Tone::Amber,
                                        )
                                    })),
                            )
                            .children(best.map(|record| {
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(px(theme::font::LABEL))
                                    .text_color(theme::muted())
                                    .child(format!(
                                        "{} {} · {:.1} Mbps",
                                        record.ip, record.colo, record.median
                                    ))
                            }))
                    })
                    .collect()
            });

        div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(header)
            .child(daily_card)
            .child(verify_card)
            .child(self.render_speed_compare(cx))
            .child(speed_card)
    }
}
