use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::time::timeout;

use crate::core::http::{self, SPEED_SNI};
use crate::core::model::SpeedRecord;

#[derive(Clone, Debug)]
pub struct SpeedConfig {
    pub targets: Vec<(String, u16)>,
    pub coarse_secs: u64,
    pub fine_secs: u64,
    pub fine_rounds: u32,
    pub fine_topn: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub enum SpeedEvent {
    Phase {
        name: &'static str,
        total: usize,
    },
    Progress {
        phase: &'static str,
        done: usize,
        total: usize,
        throttled: u32,
        current: Option<String>,
    },
    Record(SpeedRecord),
    Warning(String),
}

#[derive(Clone, Debug, Default)]
pub struct SpeedOutcome {
    pub records: Vec<SpeedRecord>,
    pub coarse_ok: usize,
    pub throttled: u32,
    pub warnings: Vec<String>,
    pub elapsed: Duration,
    pub stopped: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadResult {
    pub mbps: Option<f64>,
    pub code: Option<u16>,
    pub note: String,
}

impl DownloadResult {
    pub fn is_throttled(&self) -> bool {
        self.code == Some(429)
    }
}

pub async fn download_mbps(ip: &str, port: u16, seconds: u64, wait: Duration) -> DownloadResult {
    let overall = wait + Duration::from_secs(seconds + 15);
    let future = async {
        let mut conn = match http::connect_tls(ip, port, SPEED_SNI, true, wait).await {
            Ok(conn) => conn,
            Err(err) => {
                return DownloadResult {
                    note: format!("{err:#}"),
                    ..Default::default()
                };
            }
        };
        let request = format!(
            "GET /__down?bytes=25000000 HTTP/1.1\r\nHost: {SPEED_SNI}\r\nUser-Agent: Mozilla/5.0\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
        if let Err(err) = http::send_all(&mut conn, request.as_bytes(), wait).await {
            return DownloadResult {
                note: format!("{err:#}"),
                ..Default::default()
            };
        }
        let head = match http::read_head(&mut conn, 32 * 1024, wait * 2).await {
            Ok(head) => head,
            Err(err) => {
                return DownloadResult {
                    note: format!("{err:#}"),
                    ..Default::default()
                };
            }
        };
        let code = head.status;
        if code != 200 {
            return DownloadResult {
                code: Some(code),
                note: format!("HTTP {code}"),
                mbps: None,
            };
        }

        let mut total = head.leftover.len() as u64;
        let started = Instant::now();
        let mut buffer = vec![0u8; 256 * 1024];
        while started.elapsed() < Duration::from_secs(seconds) {
            match timeout(Duration::from_secs(8), conn.read(&mut buffer)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(read)) => total += read as u64,
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        let elapsed = started.elapsed().as_secs_f64();
        if elapsed < 0.8 || total < 256 * 1024 {
            return DownloadResult {
                code: Some(code),
                note: "too_short".to_string(),
                mbps: None,
            };
        }
        DownloadResult {
            mbps: Some((total as f64 * 8.0 / 1_000_000.0) / elapsed),
            code: Some(code),
            note: String::new(),
        }
    };

    match timeout(overall, future).await {
        Ok(result) => result,
        Err(_) => DownloadResult {
            note: "timeout".to_string(),
            ..Default::default()
        },
    }
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.len().is_multiple_of(2) {
        let mid = sorted.len() / 2;
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    }
}

async fn sleep_or_cancel(seconds: u64, cancel: &Arc<AtomicBool>) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn execute<F>(config: &SpeedConfig, cancel: Arc<AtomicBool>, mut emit: F) -> SpeedOutcome
where
    F: FnMut(SpeedEvent) + Send,
{
    let started = Instant::now();
    let mut outcome = SpeedOutcome::default();
    let total = config.targets.len();
    if total == 0 {
        return outcome;
    }

    emit(SpeedEvent::Phase {
        name: "粗筛",
        total,
    });
    let mut coarse: Vec<(String, u16, f64)> = Vec::new();
    let mut throttled = 0u32;
    let mut warned = false;

    for (ix, (ip, port)) in config.targets.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcome.stopped = true;
            break;
        }
        let result = download_mbps(ip, *port, config.coarse_secs, config.timeout).await;
        if let Some(mbps) = result.mbps {
            coarse.push((ip.clone(), *port, mbps));
        } else if result.is_throttled() {
            throttled += 1;
        }
        if throttled >= 5 && !warned {
            warned = true;
            let warning = format!(
                "已有 {throttled} 个请求被 Cloudflare 限流（HTTP 429），本轮结果可能不准确；建议 10–30 分钟后再测"
            );
            outcome.warnings.push(warning.clone());
            emit(SpeedEvent::Warning(warning));
        }
        emit(SpeedEvent::Progress {
            phase: "粗筛",
            done: ix + 1,
            total,
            throttled,
            current: Some(format!(
                "{ip}:{port} · {}",
                result
                    .mbps
                    .map(|mbps| format!("{mbps:.1} Mbps"))
                    .unwrap_or(result.note)
            )),
        });
    }

    outcome.coarse_ok = coarse.len();
    outcome.throttled = throttled;
    if !cancel.load(Ordering::Relaxed) {
        coarse.sort_by(|a, b| b.2.total_cmp(&a.2));
        let alive: Vec<&(String, u16, f64)> =
            coarse.iter().filter(|(_, _, mbps)| *mbps > 15.0).collect();
        let mut selected: Vec<(String, u16, f64)> = if alive.is_empty() {
            coarse.iter().take(config.fine_topn).cloned().collect()
        } else {
            alive
                .iter()
                .take(config.fine_topn)
                .map(|item| (**item).clone())
                .collect()
        };
        selected.truncate(config.fine_topn);

        emit(SpeedEvent::Phase {
            name: "精测",
            total: selected.len(),
        });

        for (ix, (ip, port, _)) in selected.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                outcome.stopped = true;
                break;
            }
            let colo = match http::probe_trace(ip, *port, config.timeout).await {
                Ok((colo, _)) => colo,
                Err(_) => "?".to_string(),
            };
            let mut values = Vec::new();
            for round in 0..config.fine_rounds {
                if cancel.load(Ordering::Relaxed) {
                    outcome.stopped = true;
                    break;
                }
                let result = download_mbps(ip, *port, config.fine_secs, config.timeout).await;
                if let Some(mbps) = result.mbps {
                    values.push(mbps);
                } else if result.is_throttled() {
                    throttled += 1;
                }
                emit(SpeedEvent::Progress {
                    phase: "精测",
                    done: ix + 1,
                    total: selected.len(),
                    throttled,
                    current: Some(format!(
                        "{ip} · 第 {}/{} 轮 · {}",
                        round + 1,
                        config.fine_rounds,
                        result
                            .mbps
                            .map(|mbps| format!("{mbps:.1} Mbps"))
                            .unwrap_or(result.note)
                    )),
                });
                if round + 1 < config.fine_rounds {
                    sleep_or_cancel(1, &cancel).await;
                }
            }
            if values.is_empty() {
                continue;
            }
            let record = SpeedRecord {
                ip: ip.clone(),
                port: *port,
                colo,
                median: median(&values),
                best: values.iter().copied().fold(0.0, f64::max),
                min: values.iter().copied().fold(f64::MAX, f64::min),
                rounds: values,
            };
            emit(SpeedEvent::Record(record.clone()));
            outcome.records.push(record);
            if ix + 1 < selected.len() {
                sleep_or_cancel(1, &cancel).await;
            }
        }

        if throttled >= 5 && !warned {
            let warning = format!("测速过程中累计 {throttled} 次 429 限流，排序可能不完整");
            outcome.warnings.push(warning.clone());
            emit(SpeedEvent::Warning(warning));
        }
        outcome
            .records
            .sort_by(|a, b| b.median.total_cmp(&a.median));
    } else {
        outcome.stopped = true;
    }

    outcome.throttled = throttled;
    outcome.elapsed = started.elapsed();
    if cancel.load(Ordering::Relaxed) {
        outcome.stopped = true;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::median;

    #[tokio::test]
    #[ignore = "requires network access to speed.cloudflare.com"]
    async fn download_over_rustls_gets_ok_response() {
        // speed.cloudflare.com 的任播地址；并非每个 Cloudflare IP 都提供测速端点（会返回 403）。
        // 只断言握手、HTTP 200 与数据流动：慢速网络下吞吐可能低于统计阈值（too_short）。
        let result = super::download_mbps(
            "162.159.140.220",
            443,
            4,
            std::time::Duration::from_secs(15),
        )
        .await;
        assert!(
            result.code == Some(200) || result.is_throttled(),
            "unexpected result: code {:?} note {}",
            result.code,
            result.note
        );
    }

    #[test]
    fn median_of_odd_even_and_empty() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }
}
