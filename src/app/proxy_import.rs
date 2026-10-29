//! 「导入代理软件」：用订阅节点做完隧道验证后，把通过的 IP 套到节点上，一键导入本机代理软件。
//!
//! 节点由本机订阅服务提供（见 `core::sub_server`）：客户端里保存的是本机订阅地址，
//! 之后每次更新订阅都会拿到最新一轮结果，自动优选每天跑完即自动生效。

use super::*;
use crate::core::model::DailyVerification;
use crate::core::proxy::{Endpoint, ProxyFeed};
use crate::core::sub_server::{self, FeedSource};
use crate::proxy_clients::{self, ProxyClient};
use crate::ui::widgets::ButtonKind;

impl AppState {
    /// 隧道验证用的订阅节点（与 `verify::node_for_ips` 一致：第一个 WS 节点）。
    fn proxy_template(&self) -> Option<VlessNode> {
        let node = crate::core::verify::node_for_ips(&self.verify_nodes);
        node.transport.eq_ignore_ascii_case("ws").then_some(node)
    }

    /// 页面上显示的、隧道验证通过的地址。
    fn tunnel_endpoints(&self, source: FeedSource) -> Vec<Endpoint> {
        match source {
            FeedSource::Daily => daily_endpoints(&self.daily_top_rows),
            FeedSource::Verify => self
                .verify_records
                .iter()
                .filter(|record| record.ok && record.transport.eq_ignore_ascii_case("ws"))
                .map(|record| Endpoint {
                    ip: record.ip.clone(),
                    port: record.port,
                    colo: record.colo.clone().unwrap_or_default(),
                })
                .collect(),
        }
    }

    /// 提供给客户端的节点。自动优选的本轮结果为空（运行失败、正在运行）时沿用上次完成的结果。
    fn proxy_feed(&self, source: FeedSource) -> Option<ProxyFeed> {
        let (endpoints, updated) = match source {
            FeedSource::Daily => {
                let rows = if self.daily_top_rows.is_empty() {
                    &self.daily_state.last_top
                } else {
                    &self.daily_top_rows
                };
                (daily_endpoints(rows), self.daily_state.last_run.clone())
            }
            FeedSource::Verify => (
                self.tunnel_endpoints(source),
                self.history
                    .verify
                    .first()
                    .map(|run| run.time.clone())
                    .unwrap_or_default(),
            ),
        };
        ProxyFeed::new(source.title(), updated, self.proxy_template()?, endpoints)
    }

    fn feed_job(source: FeedSource) -> JobKind {
        match source {
            FeedSource::Daily => JobKind::Daily,
            FeedSource::Verify => JobKind::Verify,
        }
    }

    /// 把最新结果交给本机订阅服务；正在运行的任务结果还不完整，保留上一份。
    /// 在任务结束、订阅载入、结果重置或重新读取后调用。
    pub(super) fn publish_proxy_feeds(&self) {
        for source in [FeedSource::Daily, FeedSource::Verify] {
            if self.job != Some(Self::feed_job(source)) {
                sub_server::publish(source, self.proxy_feed(source));
            }
        }
    }

    /// 启动本机订阅服务（首次使用时生成令牌），返回端口以及端口是否因占用而改变。
    pub(super) fn start_feed_server(&mut self) -> Result<(u16, bool), String> {
        if self.settings.proxy_feed_token.is_empty() {
            self.settings.proxy_feed_token = format!("{:032x}", rand::random::<u128>());
        }
        let saved = self.settings.proxy_feed_port;
        let port = sub_server::start(jobs::runtime(), saved, &self.settings.proxy_feed_token)?;
        self.settings.proxy_feed_port = port;
        self.save_settings();
        Ok((port, saved != 0 && saved != port))
    }

    /// 启动时恢复本机订阅服务，让已导入的客户端可以继续更新。
    pub(super) fn resume_feed_server(&mut self) {
        if !self.settings.proxy_feed_enabled {
            return;
        }
        let saved = self.settings.proxy_feed_port;
        let (text, tone) = match self.start_feed_server() {
            Ok((port, true)) => (
                format!("本机订阅端口 {saved} 被占用，已改用 {port}：请在代理软件中重新导入一次"),
                Tone::Amber,
            ),
            Ok(_) => return,
            Err(err) => (
                format!("{err}；代理软件暂时无法更新 CF Best 订阅"),
                Tone::Amber,
            ),
        };
        storage::log_line(&text);
        self.status = StatusLine::new(text, tone);
        self.status_history.push_front(self.status.clone());
    }

    /// 准备好订阅地址：结果可用、服务已启动且已收到最新结果。
    fn prepare_feed(&mut self, source: FeedSource) -> Result<(ProxyFeed, String, u16), String> {
        let feed = self
            .proxy_feed(source)
            .ok_or("没有可导入的节点：需要隧道验证通过的 IP，以及已载入的 VLESS + WS 订阅节点")?;
        let (port, _) = self.start_feed_server()?;
        if !self.settings.proxy_feed_enabled {
            self.settings.proxy_feed_enabled = true;
            self.save_settings();
        }
        self.publish_proxy_feeds();
        let token = self.settings.proxy_feed_token.clone();
        Ok((feed, token, port))
    }

    fn import_to_client(
        &mut self,
        source: FeedSource,
        client: &ProxyClient,
        cx: &mut Context<Self>,
    ) {
        let (feed, token, port) = match self.prepare_feed(source) {
            Ok(prepared) => prepared,
            Err(err) => return self.fail(err, cx),
        };
        let url = sub_server::feed_url(port, &token, source, Some(client.format));
        match proxy_clients::open_url(&client.import_link(&url, source.title())) {
            Ok(()) => {
                storage::log_line(&format!(
                    "已请求 {} 导入{}（{} 个节点）",
                    client.name,
                    source.title(),
                    feed.endpoints.len()
                ));
                self.notify_status(
                    format!(
                        "已打开 {}：在客户端中确认导入「{}」（{} 个节点）",
                        client.name,
                        source.title(),
                        feed.endpoints.len()
                    ),
                    Tone::Green,
                    cx,
                );
                self.show_toast(
                    format!("已发送到 {}，请在客户端中确认", client.name),
                    Tone::Green,
                    cx,
                );
            }
            Err(err) => self.fail(format!("打开 {} 失败：{err}", client.name), cx),
        }
    }

    fn copy_feed_url(&mut self, source: FeedSource, cx: &mut Context<Self>) {
        let (_, token, port) = match self.prepare_feed(source) {
            Ok(prepared) => prepared,
            Err(err) => return self.fail(err, cx),
        };
        self.copy_text(
            sub_server::feed_url(port, &token, source, None),
            "订阅地址",
            cx,
        );
        self.notify_status(
            "已复制本机订阅地址：在代理软件中添加为订阅（v2rayN、NekoBox 等），更新订阅即可同步最新结果",
            Tone::Green,
            cx,
        );
    }

    fn copy_feed_links(&mut self, source: FeedSource, cx: &mut Context<Self>) {
        let Some(feed) = self.proxy_feed(source) else {
            return self.fail(
                "没有可导入的节点：需要隧道验证通过的 IP，以及已载入的订阅节点",
                cx,
            );
        };
        let links = feed.share_links();
        let count = links.len();
        self.copy_text(links.join("\n"), &format!(" {count} 条节点链接"), cx);
    }

    /// 结果卡片底部的「导入代理软件」：只在有隧道验证通过的结果时出现。
    pub(super) fn render_proxy_import(
        &self,
        source: FeedSource,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let count = self.tunnel_endpoints(source).len();
        if count == 0 {
            return None;
        }
        let id = match source {
            FeedSource::Daily => "proxy-import-daily",
            FeedSource::Verify => "proxy-import-verify",
        };
        let running = self.job == Some(Self::feed_job(source));
        let template = self.proxy_template();

        let summary = match &template {
            Some(node) if !node.name.trim().is_empty() => {
                format!(
                    "{count} 个隧道验证通过的 IP × 订阅节点「{}」",
                    node.name.trim()
                )
            }
            _ => format!("{count} 个隧道验证通过的 IP"),
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(widgets::icon("zap", 14.0, theme::accent()))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::font::BODY_LG))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .child("导入代理软件"),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::font::SMALL))
                    .text_color(theme::faint())
                    .child(summary),
            );

        let actions = if template.is_none() {
            // 订阅没载入（常见于订阅地址暂时连不上）：节点信息不全，先重新拉取。
            let loading = self.sub_loading;
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    widgets::button_with_state(
                        (id, 0usize),
                        if loading {
                            "正在拉取订阅…"
                        } else {
                            "重新拉取订阅"
                        },
                        "refresh",
                        ButtonKind::Secondary,
                        loading,
                    )
                    .on_click(cx.listener(Self::load_subscription)),
                )
                .child(widgets::hint("订阅节点未载入，暂时无法生成节点。"))
        } else {
            let clients = self.proxy_clients.iter().enumerate().map(|(ix, client)| {
                let client = client.clone();
                widgets::button_with_state(
                    (id, ix + 1),
                    client.name.clone(),
                    "download",
                    ButtonKind::Secondary,
                    running,
                )
                .tooltip(crate::ui::tooltip::tooltip(
                    format!("在 {} 中导入「{}」", client.name, source.title()),
                    None,
                ))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.job == Some(Self::feed_job(source)) {
                        return this.notify_status("任务完成后再导入", Tone::Muted, cx);
                    }
                    this.import_to_client(source, &client, cx);
                }))
            });
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .children(clients)
                .when(self.proxy_clients.is_empty(), |row| {
                    row.child(widgets::hint(
                        "未检测到支持一键导入的客户端，可复制订阅地址手动添加。",
                    ))
                })
                .child(div().flex_1())
                .child(
                    widgets::button((id, 100usize), "复制订阅地址", "link", ButtonKind::Ghost)
                        .tooltip(crate::ui::tooltip::tooltip(
                            "本机订阅地址：v2rayN、NekoBox 等客户端可添加为订阅",
                            None,
                        ))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.copy_feed_url(source, cx)
                        })),
                )
                .child(
                    widgets::button((id, 101usize), "复制节点链接", "copy", ButtonKind::Ghost)
                        .tooltip(crate::ui::tooltip::tooltip(
                            "vless:// 分享链接，可在客户端中从剪贴板导入",
                            None,
                        ))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.copy_feed_links(source, cx)
                        })),
                )
        };

        let note = match source {
            FeedSource::Daily => {
                "导入后，客户端每次更新订阅都会拿到最新一轮自动优选的结果；订阅由本机提供，更新时需保持 CF Best 运行（可开启托盘常驻）。"
            }
            FeedSource::Verify => {
                "导入后，重新验证再在客户端中更新订阅即可同步；订阅由本机提供，更新时需保持 CF Best 运行。"
            }
        };

        Some(
            div()
                .flex()
                .flex_col()
                .gap_2p5()
                .p_3()
                .rounded(px(theme::radius::LG))
                .bg(theme::panel_alt())
                .child(header)
                .child(actions)
                .child(widgets::hint(note))
                .into_any_element(),
        )
    }
}

fn daily_endpoints(rows: &[DailyEntry]) -> Vec<Endpoint> {
    rows.iter()
        .filter(|entry| entry.verification == DailyVerification::Tunnel)
        .map(|entry| Endpoint {
            ip: entry.ip.clone(),
            port: entry.port,
            colo: entry.colo.clone(),
        })
        .collect()
}
