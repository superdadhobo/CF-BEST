//! 检查更新：状态栏右下角的版本号。自动检查（启动后与每天一次）发现新版本时在旁边提示，
//! 点开弹层可以看更新内容，一键下载（测速选线、校验 SHA-256）并更新。

use super::*;
use crate::core::update::{self, DownloadProgress, Release};
use crate::ui::widgets::ButtonKind;
use crate::updater::{self, Installed};

/// 自动检查的间隔。
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Default)]
pub(super) enum UpdateState {
    /// 本次运行还没检查过。
    #[default]
    Idle,
    Checking {
        manual: bool,
    },
    UpToDate {
        checked_at: String,
    },
    Available(Release),
    Downloading {
        release: Release,
        progress: DownloadProgress,
    },
    /// 已下载并校验，等正在运行的任务结束后重启（Windows）。
    Ready {
        release: Release,
        bytes: Arc<Vec<u8>>,
    },
    /// 安装包已保存：macOS 已打开 dmg；Windows 程序目录不可写时存到「下载」。
    Saved {
        release: Release,
        path: std::path::PathBuf,
    },
    Failed {
        message: String,
        release: Option<Release>,
    },
}

/// 当前版本。调试构建可以用 `CFBEST_FAKE_VERSION` 冒充旧版本，测试更新流程。
pub(super) fn current_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(version) = std::env::var("CFBEST_FAKE_VERSION") {
        return version;
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

impl AppState {
    /// 自动检查：开启时启动后第一次心跳（约 10 秒）检查一次，之后每 24 小时一次。
    pub(super) fn maybe_check_update(&mut self, cx: &mut Context<Self>) {
        if !self.settings.update_check || self.update_in_progress() {
            return;
        }
        if self
            .last_update_check
            .is_some_and(|at| at.elapsed() < CHECK_INTERVAL)
        {
            return;
        }
        self.start_update_check(false, cx);
    }

    fn update_in_progress(&self) -> bool {
        matches!(
            self.update,
            UpdateState::Checking { .. }
                | UpdateState::Downloading { .. }
                | UpdateState::Ready { .. }
        )
    }

    fn start_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        self.last_update_check = Some(Instant::now());
        self.update = UpdateState::Checking { manual };
        jobs::spawn_update_check(self.events_tx.clone());
        cx.notify();
    }

    pub(super) fn on_update_checked(
        &mut self,
        result: Result<Release, String>,
        cx: &mut Context<Self>,
    ) {
        let manual = matches!(self.update, UpdateState::Checking { manual: true });
        let current = current_version();
        match result {
            Ok(release) if update::is_newer(&release.version, &current) => {
                self.notify_in_background(
                    "CF Best 有新版本",
                    &format!(
                        "v{} 已发布，打开窗口后点右下角的版本号即可更新",
                        release.version
                    ),
                );
                self.notify_status(
                    format!("发现新版本 v{}：点右下角的版本号即可更新", release.version),
                    Tone::Accent,
                    cx,
                );
                self.update = UpdateState::Available(release);
                #[cfg(debug_assertions)]
                if std::env::var_os("CFBEST_AUTO_UPDATE").is_some() {
                    self.start_update_download(cx);
                }
            }
            Ok(_) => {
                self.update = UpdateState::UpToDate {
                    checked_at: Local::now().format("%H:%M").to_string(),
                };
                if manual {
                    self.notify_status(format!("已是最新版本 v{current}"), Tone::Green, cx);
                }
            }
            Err(err) => {
                let message = format!("检查更新失败：{err}");
                if manual {
                    self.fail(message.clone(), cx);
                }
                self.update = UpdateState::Failed {
                    message,
                    release: None,
                };
            }
        }
        cx.notify();
    }

    fn start_update_download(&mut self, cx: &mut Context<Self>) {
        let release = match &self.update {
            UpdateState::Available(release)
            | UpdateState::Failed {
                release: Some(release),
                ..
            } => release.clone(),
            _ => return,
        };
        let Some(asset) = release.platform_asset().cloned() else {
            return self.open_release_page(&release.page, cx);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.update_cancel = Some(cancel.clone());
        self.update = UpdateState::Downloading {
            progress: DownloadProgress {
                total: asset.size,
                racing: true,
                ..Default::default()
            },
            release,
        };
        jobs::spawn_update_download(asset, cancel, self.events_tx.clone());
        self.notify_status("正在下载新版本：先测速选出最快的线路…", Tone::Accent, cx);
    }

    pub(super) fn on_update_progress(&mut self, progress: DownloadProgress) {
        if let UpdateState::Downloading {
            progress: current, ..
        } = &mut self.update
        {
            *current = progress;
        }
    }

    pub(super) fn on_update_downloaded(
        &mut self,
        result: Result<Vec<u8>, String>,
        cx: &mut Context<Self>,
    ) {
        let cancelled = self
            .update_cancel
            .take()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        let UpdateState::Downloading { release, .. } = self.update.clone() else {
            return;
        };
        match result {
            // 重启会打断正在运行的任务：先留着，等用户在任务结束后点「重启完成更新」。
            Ok(bytes) if cfg!(target_os = "windows") && self.busy() => {
                self.notify_status(
                    format!(
                        "v{} 已下载并校验：当前任务结束后，点右下角的版本号重启完成更新",
                        release.version
                    ),
                    Tone::Green,
                    cx,
                );
                self.update = UpdateState::Ready {
                    release,
                    bytes: Arc::new(bytes),
                };
            }
            Ok(bytes) => self.install_update(release, &bytes, cx),
            Err(_) if cancelled => {
                self.update = UpdateState::Available(release);
                self.notify_status("已取消下载新版本", Tone::Muted, cx);
            }
            Err(err) => {
                let message = format!("下载新版本失败：{err}");
                self.fail(message.clone(), cx);
                self.update = UpdateState::Failed {
                    message,
                    release: Some(release),
                };
            }
        }
        cx.notify();
    }

    fn install_update(&mut self, release: Release, bytes: &[u8], cx: &mut Context<Self>) {
        let name = release
            .platform_asset()
            .map(|asset| asset.name.clone())
            .unwrap_or_else(|| format!("CFBest-{}", release.version));
        match updater::install(&name, bytes) {
            Ok(Installed::Restarting) => {
                storage::log_line(&format!("更新到 v{}，正在重启", release.version));
                // 先移除托盘图标，否则进程退出后图标会残留到鼠标划过为止。
                self.tray = None;
                cx.quit();
            }
            Ok(Installed::Saved(path)) => {
                let text = if cfg!(target_os = "macos") {
                    "已打开安装包：把 CF Best 拖进「应用程序」替换旧版本即可".to_string()
                } else {
                    format!("程序所在的文件夹不可写，新版本已保存到 {}", path.display())
                };
                self.notify_status(text, Tone::Green, cx);
                self.update = UpdateState::Saved { release, path };
            }
            Err(err) => {
                let message = format!("安装新版本失败：{err}");
                self.fail(message.clone(), cx);
                self.update = UpdateState::Failed {
                    message,
                    release: Some(release),
                };
            }
        }
    }

    fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return self.notify_status("有任务在运行，结束后再重启完成更新", Tone::Amber, cx);
        }
        if let UpdateState::Ready { release, bytes } = self.update.clone() {
            self.install_update(release, &bytes, cx);
        }
    }

    fn open_release_page(&mut self, page: &str, cx: &mut Context<Self>) {
        if let Err(err) = crate::proxy_clients::open_url(page) {
            self.fail(format!("无法打开发布页：{err}"), cx);
        }
    }

    fn toggle_update_popover(&mut self, cx: &mut Context<Self>) {
        let just_closed = self
            .update_popover_closed_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250));
        self.update_popover_open = !self.update_popover_open && !just_closed;
        // 还没检查过（例如关闭了自动检查）时，打开就顺手查一次。
        if self.update_popover_open && matches!(self.update, UpdateState::Idle) {
            self.start_update_check(true, cx);
        }
        cx.notify();
    }

    /// 状态栏右下角的版本号：点开检查更新；有新版本、下载中或待重启时旁边显示提示。
    pub(super) fn render_version_label(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let badge = match &self.update {
            UpdateState::Available(release)
            | UpdateState::Failed {
                release: Some(release),
                ..
            } => Some(format!("新版本 v{}", release.version)),
            UpdateState::Downloading { progress, .. } if progress.total > 0 => Some(format!(
                "下载中 {:.0}%",
                100.0 * progress.done as f64 / progress.total as f64
            )),
            UpdateState::Downloading { .. } => Some("下载中".into()),
            UpdateState::Ready { .. } => Some("待重启更新".into()),
            _ => None,
        };
        let popover = self
            .update_popover_open
            .then(|| self.render_update_popover(cx));
        div()
            .id("version-label")
            .relative()
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(22.))
            .px_1p5()
            .mr(px(-6.))
            .rounded(px(theme::radius::SM))
            .cursor_pointer()
            .hover(|style| style.bg(theme::row_hover()))
            .tooltip(crate::ui::tooltip::tooltip("检查更新", None))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_update_popover(cx)))
            .child(widgets::icon("logo", 14.0, theme::faint()))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::font::CAPTION))
                    .text_color(theme::faint())
                    .child(format!("v{}", current_version())),
            )
            .children(badge.map(|text| {
                div()
                    .flex()
                    .items_center()
                    .h(px(18.))
                    .px_1p5()
                    .rounded(px(theme::radius::XS))
                    .bg(Tone::Accent.bg())
                    .text_color(Tone::Accent.color())
                    .text_size(px(theme::font::MICRO))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(text)
            }))
            .children(popover)
            .into_any_element()
    }

    fn render_update_popover(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let body = self.render_update_body(cx);
        let auto = self.settings.update_check;
        let panel = div()
            .id("update-popover")
            .w(px(400.))
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(theme::radius::XL))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .shadow(theme::shadow::popover())
            .cursor_default()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.update_popover_open = false;
                this.update_popover_closed_at = Some(Instant::now());
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::icon("sparkles", 14.0, theme::muted()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font::BODY_LG))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child("检查更新"),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px(theme::font::CAPTION))
                            .text_color(theme::faint())
                            .child(format!("当前 v{}", current_version())),
                    ),
            )
            .child(body)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme::border_subtle())
                    .child(
                        div()
                            .id("update-auto-check")
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.settings.update_check = !this.settings.update_check;
                                this.save_settings();
                                cx.notify();
                            }))
                            .child(widgets::switch(auto))
                            .child(
                                div()
                                    .text_size(px(theme::font::BODY))
                                    .text_color(theme::text())
                                    .child("自动检查"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font::SMALL))
                            .text_color(theme::faint())
                            .child("启动时与每天一次，只向 GitHub 查询版本信息"),
                    ),
            );
        div()
            .absolute()
            .top_0()
            .right_0()
            .child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Corner::BottomRight)
                        .offset(gpui::point(px(8.), px(-6.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(panel),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    fn render_update_body(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let line = |icon: &'static str, color: gpui::Rgba, text: String| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(widgets::icon(icon, 14.0, color))
                .child(
                    div()
                        .text_size(px(theme::font::BODY_LG))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(text),
                )
        };
        let column = || div().flex().flex_col().gap_2p5();
        let buttons = || div().flex().flex_wrap().items_center().gap_2();
        let release_page = |page: String| {
            widgets::button("update-page", "发布页", "link", ButtonKind::Ghost).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.open_release_page(&page, cx)),
            )
        };

        match self.update.clone() {
            UpdateState::Idle => column().child(widgets::hint("还没有检查过新版本。")).child(
                buttons().child(
                    widgets::button("update-check", "检查更新", "refresh", ButtonKind::Primary)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_update_check(true, cx)
                        })),
                ),
            ),
            UpdateState::Checking { .. } => column().child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::spinner("update-spinner", 14.0, theme::accent()))
                    .child(
                        div()
                            .text_size(px(theme::font::BODY))
                            .text_color(theme::muted())
                            .child("正在向 GitHub 查询最新版本…"),
                    ),
            ),
            UpdateState::UpToDate { checked_at } => column()
                .child(line(
                    "check-circle",
                    theme::green(),
                    format!("已是最新版本（{checked_at} 检查）"),
                ))
                .child(
                    buttons().child(
                        widgets::button("update-recheck", "重新检查", "refresh", ButtonKind::Ghost)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.start_update_check(true, cx)
                            })),
                    ),
                ),
            UpdateState::Available(release) => {
                let installable = release.platform_asset().is_some();
                column()
                    .child(line(
                        "sparkles",
                        theme::accent(),
                        format!("发现新版本 v{}", release.version),
                    ))
                    .children(release_notes(release.changes()))
                    .child(
                        buttons()
                            .when(installable, |row| {
                                row.child(
                                    widgets::button(
                                        "update-install",
                                        if cfg!(target_os = "windows") {
                                            "立即更新"
                                        } else {
                                            "下载安装包"
                                        },
                                        "download",
                                        ButtonKind::Primary,
                                    )
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.start_update_download(cx)
                                    })),
                                )
                            })
                            .child(release_page(release.page.clone())),
                    )
                    .child(widgets::hint(if cfg!(target_os = "windows") {
                        "同时测速 GitHub 直连与 gh-proxy.com 等镜像，走最快的线路下载，用 GitHub 给出的 SHA-256 校验后自动重启到新版本。"
                    } else {
                        "同时测速 GitHub 直连与 gh-proxy.com 等镜像，走最快的线路下载，用 GitHub 给出的 SHA-256 校验后打开安装包。"
                    }))
            }
            UpdateState::Downloading { release, progress } => {
                let fraction = if progress.total > 0 {
                    progress.done as f32 / progress.total as f32
                } else {
                    0.0
                };
                let speed = if progress.speed > 0.0 {
                    format!(" · {}/s", megabytes(progress.speed as u64))
                } else {
                    String::new()
                };
                let amount = if progress.total > 0 {
                    format!(
                        "{} / {}",
                        megabytes(progress.done),
                        megabytes(progress.total)
                    )
                } else {
                    megabytes(progress.done)
                };
                let route = if progress.route.is_empty() {
                    "正在连接下载线路…".to_string()
                } else if progress.racing {
                    format!("测速中，领先：{}{speed}", progress.route)
                } else {
                    format!("经 {} 下载{speed}", progress.route)
                };
                column()
                    .child(line(
                        "download",
                        theme::accent(),
                        format!("正在下载 v{}", release.version),
                    ))
                    .child(widgets::progress_bar("update-progress", fraction, true))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .text_size(px(theme::font::SMALL))
                            .text_color(theme::muted())
                            .child(route)
                            .child(div().font_family(theme::FONT_MONO).child(amount)),
                    )
                    .child(
                        buttons().child(
                            widgets::button("update-cancel", "取消", "x-circle", ButtonKind::Ghost)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, _| {
                                    if let Some(cancel) = &this.update_cancel {
                                        cancel.store(true, Ordering::Relaxed);
                                    }
                                })),
                        ),
                    )
            }
            UpdateState::Ready { release, .. } => {
                let busy = self.busy();
                column()
                    .child(line(
                        "check-circle",
                        theme::green(),
                        format!("v{} 已下载并校验", release.version),
                    ))
                    .child(widgets::hint(if busy {
                        "有任务在运行，重启会中断它；任务结束后再重启完成更新。"
                    } else {
                        "重启后即为新版本，当前设置与数据都会保留。"
                    }))
                    .child(
                        buttons().child(
                            widgets::button_with_state(
                                "update-restart",
                                "重启完成更新",
                                "refresh",
                                ButtonKind::Primary,
                                busy,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.restart_to_update(cx)
                                }),
                            ),
                        ),
                    )
            }
            UpdateState::Saved { release, path } => {
                let folder = path.parent().map(std::path::Path::to_path_buf);
                column()
                    .child(line(
                        "check-circle",
                        theme::green(),
                        if cfg!(target_os = "macos") {
                            format!("已打开 v{} 的安装包", release.version)
                        } else {
                            format!("v{} 已保存", release.version)
                        },
                    ))
                    .child(widgets::hint(if cfg!(target_os = "macos") {
                        "把 CF Best 拖进「应用程序」替换旧版本，再重新打开即可。".to_string()
                    } else {
                        format!("程序所在的文件夹不可写，新版本保存在 {}", path.display())
                    }))
                    .child(buttons().children(folder.map(|folder| {
                        widgets::button(
                            "update-folder",
                            "打开所在文件夹",
                            "folder-open",
                            ButtonKind::Ghost,
                        )
                        .on_click(
                            cx.listener(move |_, _: &ClickEvent, _, _| {
                                storage::open_folder(&folder)
                            }),
                        )
                    })))
            }
            UpdateState::Failed { message, release } => {
                let page = release.as_ref().map_or_else(
                    || format!("https://github.com/{}/releases/latest", update::REPO),
                    |release| release.page.clone(),
                );
                let retry_download = release.is_some();
                column()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(div().mt(px(2.)).child(widgets::icon(
                                "alert-triangle",
                                14.0,
                                theme::red(),
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(theme::font::BODY))
                                    .line_height(px(20.))
                                    .text_color(theme::red())
                                    .child(message),
                            ),
                    )
                    .child(
                        buttons()
                            .child(
                                widgets::button(
                                    "update-retry",
                                    "重试",
                                    "refresh",
                                    ButtonKind::Primary,
                                )
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        if retry_download {
                                            this.start_update_download(cx);
                                        } else {
                                            this.start_update_check(true, cx);
                                        }
                                    },
                                )),
                            )
                            .child(release_page(page)),
                    )
            }
        }
    }
}

/// 更新内容（发布页正文的 Markdown）：小标题、列表项与普通段落，过长时在框内滚动。
fn release_notes(text: &str) -> Option<gpui::AnyElement> {
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.trim().replace(['`'], "").replace("**", ""))
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let rows = lines.into_iter().map(|line| {
        if let Some(title) = line
            .strip_prefix('#')
            .map(|rest| rest.trim_start_matches('#').trim())
        {
            div()
                .pt_1()
                .text_size(px(theme::font::BODY))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(title.to_string())
        } else {
            let (bullet, text) = match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                Some(item) => ("•", item.to_string()),
                None => ("", line),
            };
            // 滚动容器里的行默认按内容撑宽：`min_w_0` 让长句换行而不是被裁掉。
            div()
                .flex()
                .items_start()
                .gap_1p5()
                .w_full()
                .min_w_0()
                .text_size(px(theme::font::SMALL))
                .line_height(px(19.))
                .text_color(theme::muted())
                .when(!bullet.is_empty(), |row| row.child(bullet))
                .child(div().flex_1().min_w_0().child(text))
        }
    });
    Some(
        div()
            .id("update-notes")
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(210.))
            .overflow_y_scroll()
            .p_2p5()
            .rounded(px(theme::radius::MD))
            .bg(theme::panel_alt())
            .children(rows)
            .into_any_element(),
    )
}
