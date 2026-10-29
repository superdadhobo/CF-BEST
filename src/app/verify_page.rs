use std::ops::Range;

use gpui::{Context, div, prelude::*};

use crate::theme::px;

use super::columns::TableId;
use super::{AppState, Page};
use crate::core::sub_server::FeedSource;
use crate::jobs::JobKind;
use crate::theme;
use crate::ui::widgets::{self, ButtonKind, Tone};

impl AppState {
    pub fn render_verify_page(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let advanced = self.settings.advanced_mode;
        let running = self.job == Some(JobKind::Verify);
        let busy = self.busy();
        let progress = self.verify_progress.clone();
        let passed = self
            .verify_records
            .iter()
            .filter(|record| record.ok)
            .count();
        let records = self.verify_records.len();
        let speed_targets = self.speed_records.len().min(20);
        let status = if running {
            ("验证中", Tone::Accent)
        } else if records > 0 {
            ("已完成", Tone::Green)
        } else {
            ("就绪", Tone::Muted)
        };
        // 与 verify::node_for_ips 一致：只有 WS 节点用于验证 IP，其余传输改用内置探测。
        let ws_node = self
            .verify_nodes
            .iter()
            .find(|node| node.transport.eq_ignore_ascii_case("ws"));
        let has_ws_node = ws_node.is_some();
        let node_host = if let Some(node) = ws_node {
            if self.verify_nodes.len() > 1 {
                format!("{} 个订阅节点", self.verify_nodes.len())
            } else if node.host.trim().is_empty() {
                node.addr.clone()
            } else {
                node.host.clone()
            }
        } else if self.sub_loading {
            "正在拉取订阅…".to_string()
        } else {
            "TLS 基础验证".to_string()
        };

        let simple_top = (!advanced).then(|| {
            let summary = div()
                .flex()
                .items_center()
                .gap_3()
                .p_3()
                .rounded(px(theme::radius::LG))
                .bg(theme::panel_alt())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::icon("gauge", 15.0, theme::muted()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_size(px(theme::font::CAPTION))
                                        .text_color(theme::faint())
                                        .child("待验证"),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::font::SUBTITLE))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(format!("{speed_targets} 个测速结果")),
                                ),
                        ),
                )
                .child(div().w(px(1.)).h(px(28.)).bg(theme::border()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::icon("key", 15.0, theme::muted()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_size(px(theme::font::CAPTION))
                                        .text_color(theme::faint())
                                        .child("验证节点"),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::font::SUBTITLE))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(node_host.clone()),
                                ),
                        ),
                )
                .child(div().flex_1())
                .child(
                    widgets::button("open-node-advanced", "订阅设置", "sliders", ButtonKind::Ghost)
                        .on_click(cx.listener(Self::toggle_advanced)),
                );

            let action = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    widgets::button_with_state("verify-speed-results", "验证测速结果", "shield-check", ButtonKind::Primary, busy || speed_targets == 0)
                        .on_click(cx.listener(Self::start_verify)),
                )
                .when(speed_targets == 0, |row| {
                    // 已有扫描结果时直接开始测速，否则只跳到测速页。
                    let has_hits = !self.scan_hits.is_empty();
                    row.child(
                        widgets::button(
                            "go-speed",
                            if has_hits { "测速扫描结果" } else { "去测速" },
                            "gauge",
                            ButtonKind::Secondary,
                        )
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                if has_hits {
                                    this.next_to_speed(event, window, cx);
                                } else {
                                    this.switch_page(Page::Speed, window, cx);
                                }
                            },
                        )),
                    )
                });

            let card = widgets::card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(widgets::section_title("shield-check", "验证可用性"))
                        .child(widgets::pill(status.0, status.1)),
                )
                .child(widgets::hint(if has_ws_node {
                    "使用订阅节点验证测速结果中最快的 IP。适用于 Cloudflare Workers / Pages 搭建的 VLESS + WS 节点。"
                } else {
                    "没有 VLESS + WS 节点时只做 TLS 基础验证；填写订阅链接后，可通过节点进行完整的隧道验证。"
                }))
                .child(summary)
                .child(action)
                .when(speed_targets == 0, |this| {
                    this.child(widgets::hint(
                        "还没有测速结果，请先完成上一步测速，或到「测速排行」使用官方候选池。",
                    ))
                })
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
                                .child(div().text_size(px(theme::font::LABEL)).text_color(theme::muted()).child(
                                    if progress.label.is_empty() {
                                        "尚未开始验证".to_string()
                                    } else {
                                        progress.label.clone()
                                    },
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
                            "verify-progress-simple",
                            progress.fraction(),
                            running,
                        )),
                )
                .when(records == 0 && !running, |this| {
                    this.child(widgets::inline_guide([
                        "完成测速",
                        "开始验证",
                        "导出结果",
                    ]))
                });

            div().w_full().child(card)
        });

        let node_card = advanced.then(|| {
            widgets::card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(widgets::section_title("link", "订阅节点"))
                        .child(widgets::hint("粘贴一个订阅链接即可")),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_3()
                        .child(widgets::field("订阅链接", self.sub_url.clone()))
                        .child(
                            widgets::button_with_state("load-subscription", if self.sub_loading { "拉取中…" } else { "拉取订阅" }, "refresh", ButtonKind::Secondary, self.sub_loading)
                                .on_click(cx.listener(Self::load_subscription)),
                        ),
                )
                .child(widgets::hint(
                    "适用于 Cloudflare Workers / Pages 搭建的 VLESS + WS 节点；其他传输方式仅做 TLS 基础验证。",
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::pill(
                            format!("订阅节点 {}", self.verify_nodes.len()),
                            if self.verify_nodes.is_empty() {
                                Tone::Muted
                            } else {
                                Tone::Green
                            },
                        ))
                        .child(
                            div()
                                .text_size(px(theme::font::LABEL))
                                .text_color(theme::faint())
                                .truncate()
                                .child(self.verify_node_source.clone()),
                        ),
                )
                .when(self.verify_nodes.is_empty(), |this| {
                    this.child(widgets::hint(
                        "未配置订阅时只做 TLS 基础验证；填写订阅链接并拉取后，将自动通过 VLESS + WS 节点进行完整的隧道验证。",
                    ))
                })
        });

        let advanced_target_card = advanced.then(|| {
            widgets::card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(widgets::section_title("shield-check", "验证目标"))
                        .child(widgets::pill(status.0, status.1)),
                )
                .child(self.verify_input.clone())
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            widgets::button_with_state(
                                "start-verify",
                                "验证 IP 列表",
                                "shield-check",
                                ButtonKind::Primary,
                                busy,
                            )
                            .on_click(cx.listener(Self::start_verify)),
                        )
                        .child(
                            widgets::button_with_state(
                                "verify-subscription",
                                "验证订阅节点",
                                "layers",
                                ButtonKind::Secondary,
                                busy || self.verify_nodes.is_empty(),
                            )
                            .on_click(cx.listener(Self::verify_subscription)),
                        )
                        .child(
                            widgets::button(
                                "speed-results-to-verify",
                                "使用测速结果",
                                "gauge",
                                ButtonKind::Ghost,
                            )
                            .on_click(cx.listener(Self::load_speed_to_verify)),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_5()
                                .child(widgets::stat("通过", passed.to_string(), Tone::Green))
                                .child(widgets::stat("总数", records.to_string(), Tone::Blue))
                                .child(widgets::stat(
                                    "当前",
                                    if progress.total == 0 {
                                        "-".to_string()
                                    } else {
                                        format!("{} / {}", progress.done, progress.total)
                                    },
                                    Tone::Muted,
                                )),
                        ),
                )
                .child(widgets::progress_bar(
                    "verify-progress-adv",
                    progress.fraction(),
                    running,
                ))
                .child(widgets::progress_caption(
                    if progress.label.is_empty() {
                        "IP 列表留空时，将验证当前已拉取的订阅节点。".to_string()
                    } else {
                        progress.label.clone()
                    },
                    running,
                    "verify-spinner-adv",
                ))
        });

        let row_count = self.verify_records.len();
        let table_body = if row_count == 0 {
            widgets::empty_state(
                "badge-check",
                if running {
                    "验证进行中…"
                } else {
                    "还没有验证结果"
                },
                if running {
                    "依次检查 DNS → TLS → WebSocket → VLESS → HTTP 204"
                } else if advanced {
                    "先拉取订阅，再验证 IP 列表"
                } else {
                    "完成上一步测速后，点击「验证测速结果」"
                },
            )
            .h(px(160.))
            .into_any_element()
        } else {
            gpui::uniform_list(
                "verify-rows",
                row_count,
                cx.processor(|this: &mut AppState, range: Range<usize>, _window, cx| {
                    let widths = this.column_widths(TableId::Verify);
                    let mut rows = Vec::with_capacity(range.end - range.start);
                    for ix in range {
                        let Some(record) = this.verify_records.get(ix) else {
                            continue;
                        };
                        let address = format!("{}:{}", record.ip, record.port);
                        let detail = format!("{} · {}", record.stage, record.detail);
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
                                .group("verify-row")
                                .cursor_pointer()
                                .on_click({
                                    let address = address.clone();
                                    cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                        this.copy_text(address.clone(), &format!(" {address}"), cx);
                                    })
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .w(px(widths[0]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY_LG))
                                        .child(address)
                                        .child(widgets::copy_hint("verify-row")),
                                )
                                .child(
                                    div()
                                        .w(px(widths[1]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::LABEL))
                                        .text_color(theme::muted())
                                        .child(if record.transport.is_empty() {
                                            "tcp".to_string()
                                        } else {
                                            record.transport.clone()
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(widths[2]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY))
                                        .text_color(theme::green())
                                        .child(
                                            record.colo.clone().unwrap_or_else(|| "-".to_string()),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(widths[3]))
                                        .flex_shrink_0()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(theme::font::BODY))
                                        .text_color(theme::muted())
                                        .child(
                                            record
                                                .ms
                                                .map(|ms| format!("{ms:.0} ms"))
                                                .unwrap_or_else(|| "-".to_string()),
                                        ),
                                )
                                .child(div().w(px(widths[4])).flex_shrink_0().child(widgets::pill(
                                    if record.ok { "通过" } else { "失败" },
                                    if record.ok { Tone::Green } else { Tone::Red },
                                )))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_size(px(theme::font::LABEL))
                                        .text_color(if record.ok {
                                            theme::muted()
                                        } else {
                                            theme::red()
                                        })
                                        .child(detail),
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

        let table_actions = (passed > 0).then(|| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    widgets::button("copy-verify-ok", "复制可用 IP", "copy", ButtonKind::Ghost)
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                            let lines = this
                                .verify_records
                                .iter()
                                .filter(|record| record.ok)
                                .map(|record| {
                                    format!(
                                        "{}:{}#{}",
                                        record.ip,
                                        record.port,
                                        record.colo.as_deref().unwrap_or("?")
                                    )
                                })
                                .collect::<Vec<_>>();
                            let count = lines.len();
                            this.copy_text(lines.join("\n"), &format!(" {count} 个可用地址"), cx);
                        })),
                )
                .child(
                    widgets::button("export-picks", "导出", "download", ButtonKind::Ghost)
                        .on_click(cx.listener(Self::export_picks)),
                )
        });

        let table_card = widgets::card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::section_title("activity", "验证结果"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::pill_action(
                                format!("{passed} / {records} 通过"),
                                if records == 0 {
                                    Tone::Muted
                                } else if passed > 0 {
                                    Tone::Green
                                } else {
                                    Tone::Red
                                },
                            ))
                            .child(
                                div()
                                    .text_size(px(theme::font::SMALL))
                                    .text_color(theme::faint())
                                    // 判定标准随验证方式不同：WS 节点走完整隧道，其余只做 TLS 基础验证。
                                    .child(
                                        if self
                                            .verify_records
                                            .iter()
                                            .any(|record| record.transport.eq_ignore_ascii_case("ws"))
                                        {
                                            "WebSocket 升级（101）、VLESS 隧道与 HTTP 204 全部通过才算可用"
                                        } else {
                                            "TLS 握手与 HTTP 回源均成功即算通过，不代表代理节点可用"
                                        },
                                    ),
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
                    .child({
                        let widths = self.column_widths(TableId::Verify);
                        widgets::table_header_row()
                            .child(self.header_cell(
                                TableId::Verify,
                                0,
                                widths[0],
                                widgets::plain_header_cell("地址"),
                                cx,
                            ))
                            .child(self.header_cell(
                                TableId::Verify,
                                1,
                                widths[1],
                                widgets::plain_header_cell("传输"),
                                cx,
                            ))
                            .child(self.header_cell(
                                TableId::Verify,
                                2,
                                widths[2],
                                widgets::plain_header_cell("机房"),
                                cx,
                            ))
                            .child(self.header_cell(
                                TableId::Verify,
                                3,
                                widths[3],
                                widgets::plain_header_cell("握手"),
                                cx,
                            ))
                            .child(self.header_cell(
                                TableId::Verify,
                                4,
                                widths[4],
                                widgets::plain_header_cell("结果"),
                                cx,
                            ))
                            .child(div().flex_1().child(widgets::plain_header_cell("详情")))
                    })
                    .child(table_body),
            )
            .children(self.render_proxy_import(FeedSource::Verify, cx));

        let top_section = match (simple_top, node_card, advanced_target_card) {
            (Some(simple), _, _) => simple,
            (_, Some(node), Some(target)) => div()
                .flex()
                .flex_col()
                .gap_4()
                .w_full()
                .child(node)
                .child(target),
            _ => div(),
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(top_section)
            .child(table_card)
    }
}
