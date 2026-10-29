//! 检查更新：从 GitHub 取最新版本，各下载线路同时测速，用 GitHub 给出的 SHA-256 校验安装包。
//!
//! 版本号、更新说明与每个文件的 SHA-256 都直接来自 GitHub（校验证书）；只有安装包本身经镜像加速，
//! 因为国内直连 GitHub 下载大文件常常只有几十 KB/s。镜像是第三方服务，随时可能失效甚至篡改内容，
//! 所以下载结果必须与 GitHub 给出的大小和 SHA-256 一致才会使用。

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::core::http;

pub const REPO: &str = "wintopic/CF-BEST";
/// 镜像（`https://<镜像>/<GitHub 下载地址>`）。第三方服务时常失效或变慢，和直连一起测速，取当时最快的。
pub const MIRRORS: &[&str] = &["gh-proxy.com", "ghfast.top", "ghproxy.net"];
const CHECK_WAIT: Duration = Duration::from_secs(15);
/// 下载中单次读取的最长等待，超过就认为这条线路卡住了。
const READ_WAIT: Duration = Duration::from_secs(20);
/// 所有线路同时下载的测速时长，之后只保留已下载最多的一条。
const RACE_WINDOW: Duration = Duration::from_secs(3);
const MAX_DOWNLOAD: usize = 300 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// 不带 `v` 的版本号，例如 `0.2.0`。
    pub version: String,
    /// 发布页正文（Markdown）。
    pub notes: String,
    /// 发布页地址。
    pub page: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    /// 字节数，0 表示未知。
    pub size: u64,
    /// GitHub 上的下载地址。
    pub url: String,
    /// 小写十六进制 SHA-256，只取自 GitHub 本身。
    pub sha256: Option<String>,
}

impl Release {
    /// 当前系统对应的安装包：Windows 用 exe，macOS 用 dmg。
    pub fn platform_asset(&self) -> Option<&Asset> {
        let suffix = if cfg!(target_os = "windows") {
            "-windows-x64.exe"
        } else if cfg!(target_os = "macos") {
            "-macos-universal.dmg"
        } else {
            return None;
        };
        self.assets
            .iter()
            .find(|asset| asset.name.ends_with(suffix))
    }

    /// 发布页正文中「## 下载」之前的部分，即本版本的更新内容。
    pub fn changes(&self) -> &str {
        let notes = self.notes.trim();
        notes
            .find("\n## ")
            .map_or(notes, |end| notes[..end].trim_end())
    }
}

/// 最新的正式版。先问 GitHub API（带每个文件的 SHA-256）；API 不通或限流时，改读直连 GitHub 的
/// `SHA256SUMS.txt`（文件名里带版本号）。两条路都只连 GitHub 本身，结果可信。
pub async fn latest_release() -> Result<Release> {
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let api_error = match http::https_get(&api, false, CHECK_WAIT)
        .await
        .and_then(|body| parse_api_release(&body))
    {
        Ok(release) => return Ok(release),
        Err(err) => err,
    };
    let sums = format!("https://github.com/{REPO}/releases/latest/download/SHA256SUMS.txt");
    let body = http::https_get(&sums, false, CHECK_WAIT)
        .await
        .map_err(|err| anyhow!("无法连接 GitHub：{api_error:#}；{err:#}"))?;
    parse_sums_release(&String::from_utf8_lossy(&body))
}

/// GitHub `releases/latest` API 的返回。文件的 SHA-256 在 `digest` 字段（`sha256:<hex>`）。
pub fn parse_api_release(body: &[u8]) -> Result<Release> {
    let json: Value = serde_json::from_slice(body).context("GitHub 返回的不是 JSON")?;
    let tag = json["tag_name"]
        .as_str()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| anyhow!("GitHub 返回中没有版本号"))?;
    let assets = json["assets"]
        .as_array()
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    Some(Asset {
                        name: asset["name"].as_str()?.to_string(),
                        size: asset["size"].as_u64().unwrap_or(0),
                        url: asset["browser_download_url"].as_str()?.to_string(),
                        sha256: asset["digest"]
                            .as_str()
                            .and_then(|digest| digest.strip_prefix("sha256:"))
                            .filter(|hex| is_sha256(hex))
                            .map(str::to_ascii_lowercase),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        notes: json["body"].as_str().unwrap_or_default().to_string(),
        page: json["html_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{tag}")),
        assets,
    })
}

/// `SHA256SUMS.txt`（`sha256sum` 格式：哈希、空白、可选的 `*`、文件名），版本号取自 `CFBest-<版本>-…`。
pub fn parse_sums_release(text: &str) -> Result<Release> {
    let mut version = None;
    let mut entries = Vec::new();
    for line in text.lines() {
        let Some((hash, name)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim().trim_start_matches('*');
        if !is_sha256(hash) || name.is_empty() {
            continue;
        }
        if version.is_none() {
            version = name
                .strip_prefix("CFBest-")
                .and_then(|rest| rest.split_once('-'))
                .map(|(version, _)| version.to_string());
        }
        entries.push((hash.to_ascii_lowercase(), name.to_string()));
    }
    let version = version.ok_or_else(|| anyhow!("SHA256SUMS.txt 中没有安装包"))?;
    let assets = entries
        .into_iter()
        .map(|(sha256, name)| Asset {
            url: format!("https://github.com/{REPO}/releases/download/v{version}/{name}"),
            name,
            size: 0,
            sha256: Some(sha256),
        })
        .collect();
    Ok(Release {
        page: format!("https://github.com/{REPO}/releases/tag/v{version}"),
        version,
        notes: String::new(),
        assets,
    })
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// `candidate` 是否比 `current` 新：按数字逐段比较（`0.10.0` 比 `0.9.1` 新），
/// 数字相同时正式版比带 `-` 后缀的预发布版新。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(version: &str) -> (Vec<u64>, bool) {
        let version = version.trim().trim_start_matches('v');
        let (core, prerelease) = match version.split_once('-') {
            Some((core, _)) => (core, true),
            None => (version, false),
        };
        let parts = core
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        (parts, prerelease)
    }
    let (new, new_pre) = parse(candidate);
    let (old, old_pre) = parse(current);
    for ix in 0..new.len().max(old.len()) {
        let (a, b) = (
            new.get(ix).copied().unwrap_or(0),
            old.get(ix).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    !new_pre && old_pre
}

/// 一条下载线路。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub label: String,
    pub url: String,
}

/// GitHub 直连与各镜像。
pub fn routes(url: &str) -> Vec<Route> {
    std::iter::once(Route {
        label: "GitHub 直连".into(),
        url: url.to_string(),
    })
    .chain(MIRRORS.iter().map(|mirror| Route {
        label: (*mirror).to_string(),
        url: format!("https://{mirror}/{url}"),
    }))
    .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DownloadProgress {
    /// 领先（测速中）或正在使用的线路。
    pub route: String,
    pub done: u64,
    /// 总字节数，0 表示未知。
    pub total: u64,
    /// 字节 / 秒，按最近的采样平滑。
    pub speed: f64,
    /// 是否还在几条线路同时测速。
    pub racing: bool,
}

/// 下载并校验安装包：所有线路同时开始，几秒后只保留已下载最多的那条；它失败或校验不通过时，
/// 其余线路重新测速。`cancel` 置位即停止。
pub async fn download(
    asset: &Asset,
    cancel: Arc<AtomicBool>,
    progress: impl FnMut(DownloadProgress),
) -> Result<Vec<u8>> {
    download_with(asset, cancel, progress, RACE_WINDOW, |route, received| {
        fetch(route.url, received)
    })
    .await
}

async fn download_with<F, Fut>(
    asset: &Asset,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(DownloadProgress),
    window: Duration,
    fetch: F,
) -> Result<Vec<u8>>
where
    F: Fn(Route, Arc<AtomicU64>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>>> + Send + 'static,
{
    let expected = asset.sha256.as_deref().ok_or_else(|| {
        anyhow!("GitHub 没有给出这个文件的 SHA-256，无法确认下载完整，请到发布页手动下载")
    })?;
    let mut remaining = routes(&asset.url);
    let mut failures = Vec::new();
    while !remaining.is_empty() {
        let mut dead = Vec::new();
        let outcome = race(
            &remaining,
            asset.size,
            window,
            &cancel,
            &mut progress,
            &fetch,
            &mut dead,
        )
        .await;
        let mut lost = dead;
        match outcome {
            Race::Cancelled => bail!("已取消下载"),
            Race::Won(route, bytes) => match verify(&bytes, asset.size, expected) {
                Ok(()) => return Ok(bytes),
                Err(err) => lost.push((route, format!("{err:#}"))),
            },
            Race::Lost if lost.is_empty() => bail!("下载线路异常中止"),
            Race::Lost => {}
        }
        // 失败或校验不通过的线路不再重试；只是被中止的线路下一轮重新测速。
        for (route, err) in lost {
            failures.push(format!("{}：{err}", route.label));
            remaining.retain(|candidate| *candidate != route);
        }
    }
    bail!("所有线路都下载失败（{}）", failures.join("；"))
}

enum Race {
    Won(Route, Vec<u8>),
    Lost,
    Cancelled,
}

struct Lane {
    route: Route,
    received: Arc<AtomicU64>,
    task: Option<JoinHandle<Result<Vec<u8>>>>,
}

impl Lane {
    fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

/// 一轮测速下载。窗口期内先下完的直接胜出；窗口期结束时保留已下载最多的线路，中止其余的。
/// 出错的线路记入 `dead`。
async fn race<F, Fut>(
    routes: &[Route],
    total: u64,
    window: Duration,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(DownloadProgress),
    fetch: &F,
    dead: &mut Vec<(Route, String)>,
) -> Race
where
    F: Fn(Route, Arc<AtomicU64>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>>> + Send + 'static,
{
    let mut lanes: Vec<Lane> = routes
        .iter()
        .map(|route| {
            let received = Arc::new(AtomicU64::new(0));
            let task = tokio::spawn(fetch(route.clone(), received.clone()));
            Lane {
                route: route.clone(),
                received,
                task: Some(task),
            }
        })
        .collect();
    let abort_all = |lanes: &mut [Lane]| {
        for lane in lanes.iter_mut() {
            if let Some(task) = lane.task.take() {
                task.abort();
            }
        }
    };
    let started = Instant::now();
    let mut chosen: Option<usize> = None;
    let mut meter = Meter::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            abort_all(&mut lanes);
            return Race::Cancelled;
        }
        for ix in 0..lanes.len() {
            if !lanes[ix].task.as_ref().is_some_and(JoinHandle::is_finished) {
                continue;
            }
            let Some(task) = lanes[ix].task.take() else {
                continue;
            };
            match task.await {
                Ok(Ok(bytes)) => {
                    let route = lanes[ix].route.clone();
                    abort_all(&mut lanes);
                    return Race::Won(route, bytes);
                }
                Ok(Err(err)) => dead.push((lanes[ix].route.clone(), format!("{err:#}"))),
                Err(err) if !err.is_cancelled() => {
                    dead.push((lanes[ix].route.clone(), format!("下载任务异常：{err}")))
                }
                Err(_) => {}
            }
            if chosen == Some(ix) {
                abort_all(&mut lanes);
                return Race::Lost;
            }
        }
        let alive: Vec<usize> = (0..lanes.len())
            .filter(|&ix| lanes[ix].task.is_some())
            .collect();
        let Some(&leader) = alive.iter().max_by_key(|&&ix| lanes[ix].received()) else {
            return Race::Lost;
        };
        if chosen.is_none() && started.elapsed() >= window {
            for &ix in &alive {
                if ix != leader
                    && let Some(task) = lanes[ix].task.take()
                {
                    task.abort();
                }
            }
            chosen = Some(leader);
        }
        let shown = chosen.unwrap_or(leader);
        progress(meter.sample(
            &lanes[shown].route.label,
            lanes[shown].received(),
            total,
            chosen.is_none(),
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 下载速度：按线路分别平滑，换线路时重新计。
#[derive(Default)]
struct Meter {
    route: String,
    last: Option<(Instant, u64)>,
    speed: f64,
}

impl Meter {
    fn sample(&mut self, route: &str, done: u64, total: u64, racing: bool) -> DownloadProgress {
        let now = Instant::now();
        if self.route != route {
            self.route = route.to_string();
            self.last = None;
            self.speed = 0.0;
        }
        if let Some((at, before)) = self.last {
            let seconds = now.duration_since(at).as_secs_f64();
            if seconds > 0.0 {
                let instant = done.saturating_sub(before) as f64 / seconds;
                self.speed = if self.speed == 0.0 {
                    instant
                } else {
                    self.speed * 0.7 + instant * 0.3
                };
            }
        }
        self.last = Some((now, done));
        DownloadProgress {
            route: route.to_string(),
            done,
            total,
            speed: self.speed,
            racing,
        }
    }
}

/// 通过一条线路下载完整内容，`received` 实时记录已收到的字节数。
async fn fetch(url: String, received: Arc<AtomicU64>) -> Result<Vec<u8>> {
    let (mut conn, head) = http::open_get(&url, false, READ_WAIT).await?;
    let length = head
        .header("content-length")
        .and_then(|value| value.parse::<usize>().ok());
    let chunked = head
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    if length.is_some_and(|length| length > MAX_DOWNLOAD) {
        bail!("文件过大");
    }
    let mut body = head.leftover.clone();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        received.store(body.len() as u64, Ordering::Relaxed);
        let complete = if chunked {
            body.ends_with(b"0\r\n\r\n")
        } else {
            length.is_some_and(|length| body.len() >= length)
        };
        if complete {
            break;
        }
        if body.len() > MAX_DOWNLOAD {
            bail!("文件过大");
        }
        let read = match timeout(READ_WAIT, conn.read(&mut chunk)).await {
            Err(_) => bail!("{} 秒没有收到数据", READ_WAIT.as_secs()),
            Ok(Ok(read)) => read,
            // 对端没发 close_notify 就断开：内容是否完整交给之后的大小与 SHA-256 校验。
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => 0,
            Ok(Err(err)) => return Err(err).context("读取下载内容失败"),
        };
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    http::decode_body(&head, body)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn verify(bytes: &[u8], size: u64, sha256: &str) -> Result<()> {
    if size > 0 && bytes.len() as u64 != size {
        bail!("大小不符（{} / {size} 字节）", bytes.len());
    }
    if sha256_hex(bytes) != sha256 {
        bail!("SHA-256 不符，文件可能损坏或被篡改");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.1"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(is_newer("0.2.0", "0.2.0-beta"));
        assert!(!is_newer("0.2.0-beta", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.1", "0.1.0"));
    }

    #[test]
    fn parses_api_release_with_digests() {
        let body = r####"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/wintopic/CF-BEST/releases/tag/v0.2.0",
            "body": "### 新增\n\n- 检查更新\n\n## 下载\n\n| 系统 | 文件 |",
            "assets": [
                {"name": "CFBest-0.2.0-macos-universal.dmg", "size": 10, "browser_download_url": "https://github.com/wintopic/CF-BEST/releases/download/v0.2.0/CFBest-0.2.0-macos-universal.dmg", "digest": "sha256:241A990FE0C2EFF2E14AA0C6CC80764EF35F36CA00AFDB242431AF86A5A2A967"},
                {"name": "CFBest-0.2.0-windows-x64.exe", "size": 20, "browser_download_url": "https://github.com/wintopic/CF-BEST/releases/download/v0.2.0/CFBest-0.2.0-windows-x64.exe", "digest": null},
                {"name": "broken"}
            ]
        }"####
        .as_bytes();
        let release = parse_api_release(body).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.changes(), "### 新增\n\n- 检查更新");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(
            release.assets[0].sha256.as_deref(),
            Some("241a990fe0c2eff2e14aa0c6cc80764ef35f36ca00afdb242431af86a5a2a967")
        );
        assert_eq!(release.assets[1].sha256, None);
        let asset = release.platform_asset();
        if cfg!(target_os = "windows") {
            assert_eq!(asset.unwrap().size, 20);
        } else if cfg!(target_os = "macos") {
            assert_eq!(asset.unwrap().size, 10);
        }
        assert!(parse_api_release(b"{\"message\": \"API rate limit exceeded\"}").is_err());
        assert!(parse_api_release(b"<html>").is_err());
    }

    #[test]
    fn parses_sha256sums_fallback() {
        let text = "eb153db6e633e4971ad33a4814d45b1a6fae8f99d8255d3fdb016dc800e1b9b2  CFBest-0.1.0-windows-x64.exe\n241a990fe0c2eff2e14aa0c6cc80764ef35f36ca00afdb242431af86a5a2a967 *CFBest-0.1.0-macos-universal.dmg\nnot a line\n";
        let release = parse_sums_release(text).unwrap();
        assert_eq!(release.version, "0.1.0");
        assert_eq!(
            release.page,
            "https://github.com/wintopic/CF-BEST/releases/tag/v0.1.0"
        );
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[1].name, "CFBest-0.1.0-macos-universal.dmg");
        assert_eq!(
            release.assets[0].url,
            "https://github.com/wintopic/CF-BEST/releases/download/v0.1.0/CFBest-0.1.0-windows-x64.exe"
        );
        assert!(parse_sums_release("").is_err());
    }

    #[test]
    fn routes_put_direct_first_then_mirrors() {
        let routes = routes("https://github.com/a/b/releases/download/v1/x.exe");
        assert_eq!(routes[0].label, "GitHub 直连");
        assert_eq!(
            routes[0].url,
            "https://github.com/a/b/releases/download/v1/x.exe"
        );
        assert_eq!(
            routes[1].url,
            "https://gh-proxy.com/https://github.com/a/b/releases/download/v1/x.exe"
        );
        assert_eq!(routes.len(), 1 + MIRRORS.len());
    }

    const PAYLOAD: &[u8] = b"new CF Best build";

    fn asset() -> Asset {
        Asset {
            name: "CFBest-9.9.9-windows-x64.exe".into(),
            size: PAYLOAD.len() as u64,
            url: "https://github.com/wintopic/CF-BEST/releases/download/v9.9.9/x.exe".into(),
            sha256: Some(sha256_hex(PAYLOAD)),
        }
    }

    type FakeFetch = std::pin::Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;

    /// 按线路名模拟：`(延迟毫秒, 结果)`；结果为 None 表示失败。
    fn fake(
        plan: &'static [(&'static str, u64, Option<&'static [u8]>)],
    ) -> impl Fn(Route, Arc<AtomicU64>) -> FakeFetch {
        move |route, received| {
            let (delay, result) = plan
                .iter()
                .find(|(label, _, _)| *label == route.label)
                .map(|(_, delay, result)| (*delay, *result))
                .unwrap_or((10_000, None));
            Box::pin(async move {
                // 先报告一点进度，延迟越短的线路「跑得越快」。
                received.store(1_000_000 / (delay + 1), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                result
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| anyhow!("{} 连接失败", route.label))
            })
        }
    }

    #[tokio::test]
    async fn fastest_route_wins_and_is_verified() {
        let mut seen = Vec::new();
        let bytes = download_with(
            &asset(),
            Arc::new(AtomicBool::new(false)),
            |progress| seen.push(progress),
            Duration::from_millis(300),
            fake(&[
                ("GitHub 直连", 5_000, Some(PAYLOAD)),
                ("gh-proxy.com", 50, Some(PAYLOAD)),
                ("ghfast.top", 2_000, Some(PAYLOAD)),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(bytes, PAYLOAD);
        assert!(
            seen.iter()
                .all(|progress| progress.total == PAYLOAD.len() as u64)
        );
    }

    #[tokio::test]
    async fn tampered_or_failed_routes_fall_back_to_a_good_one() {
        let bytes = download_with(
            &asset(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
            Duration::from_millis(100),
            fake(&[
                ("gh-proxy.com", 20, Some(b"tampered build!!!")),
                ("ghfast.top", 30, None),
                ("ghproxy.net", 400, Some(PAYLOAD)),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(bytes, PAYLOAD);
    }

    #[tokio::test]
    async fn reports_every_failed_route() {
        let err = download_with(
            &asset(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
            Duration::from_millis(100),
            fake(&[
                ("GitHub 直连", 10, None),
                ("gh-proxy.com", 10, None),
                ("ghfast.top", 10, Some(b"short")),
                ("ghproxy.net", 10, None),
            ]),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.starts_with("所有线路都下载失败"), "{err}");
        for label in [
            "GitHub 直连",
            "gh-proxy.com",
            "ghfast.top：大小不符",
            "ghproxy.net",
        ] {
            assert!(err.contains(label), "{err}");
        }
    }

    #[tokio::test]
    async fn cancel_stops_and_missing_digest_refuses() {
        let cancel = Arc::new(AtomicBool::new(true));
        let err = download_with(
            &asset(),
            cancel,
            |_| {},
            Duration::from_millis(100),
            fake(&[("GitHub 直连", 2_000, Some(PAYLOAD))]),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "已取消下载");

        let unsigned = Asset {
            sha256: None,
            ..asset()
        };
        let err = download(&unsigned, Arc::new(AtomicBool::new(false)), |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SHA-256"));
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    #[ignore = "requires network access to GitHub and its mirrors"]
    async fn downloads_latest_release_through_the_fastest_route() {
        let release = latest_release().await.expect("latest release");
        let asset = release
            .platform_asset()
            .expect("asset for this platform")
            .clone();
        let mut last = DownloadProgress::default();
        let started = Instant::now();
        let bytes = download(&asset, Arc::new(AtomicBool::new(false)), |progress| {
            if progress.route != last.route {
                eprintln!(
                    "{:.1}s 领先线路：{}",
                    started.elapsed().as_secs_f64(),
                    progress.route
                );
            }
            last = progress;
        })
        .await
        .expect("download");
        eprintln!(
            "v{} {} 经 {} 下载完成：{} 字节，{:.1}s",
            release.version,
            asset.name,
            last.route,
            bytes.len(),
            started.elapsed().as_secs_f64()
        );
        assert_eq!(sha256_hex(&bytes), asset.sha256.unwrap());
    }
}
