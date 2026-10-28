use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::core::http;
use crate::core::model::ScanHit;

#[derive(Clone, Debug)]
pub struct ScanConfig {
    pub targets: Vec<(Ipv4Addr, u16)>,
    pub concurrency: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub enum ScanEvent {
    Progress {
        done: usize,
        total: usize,
        hits: usize,
        elapsed: Duration,
    },
    Hit(ScanHit),
}

#[derive(Clone, Debug, Default)]
pub struct ScanOutcome {
    pub hits: Vec<ScanHit>,
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub elapsed: Duration,
    pub stopped: bool,
}

async fn probe(
    ip: Ipv4Addr,
    port: u16,
    wait: Duration,
) -> (Ipv4Addr, u16, Result<(String, f64), String>) {
    let text = ip.to_string();
    match timeout(wait, http::probe_trace(&text, port, wait)).await {
        Ok(Ok((colo, ms))) => (ip, port, Ok((colo, ms))),
        Ok(Err(err)) => (ip, port, Err(format!("{err:#}"))),
        Err(_) => (ip, port, Err("超时".to_string())),
    }
}

pub async fn execute<F>(config: &ScanConfig, cancel: Arc<AtomicBool>, mut emit: F) -> ScanOutcome
where
    F: FnMut(ScanEvent) + Send,
{
    let started = Instant::now();
    let concurrency = config.concurrency.clamp(1, 2000);
    let total = config.targets.len();
    let mut hits = Vec::new();
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut stopped = false;
    let mut last_progress = Instant::now() - Duration::from_secs(10);

    let mut iter = config.targets.iter().copied();
    let mut tasks = JoinSet::new();
    for _ in 0..concurrency {
        let Some((ip, port)) = iter.next() else {
            break;
        };
        tasks.spawn(probe(ip, port, config.timeout));
    }

    loop {
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        done += 1;
        match joined {
            Ok((ip, port, Ok((colo, ms)))) => {
                let hit = ScanHit::new(ip.to_string(), port, colo, ms);
                emit(ScanEvent::Hit(hit.clone()));
                hits.push(hit);
            }
            Ok((_, _, Err(_))) => failed += 1,
            Err(err) if err.is_cancelled() => {}
            Err(_) => failed += 1,
        }

        if cancel.load(Ordering::Relaxed) {
            stopped = true;
            tasks.abort_all();
            break;
        }

        if let Some((ip, port)) = iter.next() {
            tasks.spawn(probe(ip, port, config.timeout));
        }

        let now = Instant::now();
        if now.duration_since(last_progress) >= Duration::from_millis(120)
            || done == total
            || done.is_multiple_of(200)
        {
            last_progress = now;
            emit(ScanEvent::Progress {
                done,
                total,
                hits: hits.len(),
                elapsed: started.elapsed(),
            });
        }
    }

    if stopped {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    hits.sort_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms));
    let outcome = ScanOutcome {
        hits,
        total,
        done,
        failed,
        elapsed: started.elapsed(),
        stopped,
    };
    emit(ScanEvent::Progress {
        done: outcome.done,
        total: outcome.total,
        hits: outcome.hits.len(),
        elapsed: outcome.elapsed,
    });
    outcome
}
