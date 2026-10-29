use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rand::RngCore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::http;
use crate::core::model::{VerifyRecord, VlessNode};

const PROBE_HOST: &str = "cp.cloudflare.com";
const PROBE_PATH: &str = "/generate_204";

#[derive(Clone, Debug)]
pub struct VerifyTarget {
    pub addr: String,
    pub port: u16,
    pub node: VlessNode,
}

#[derive(Clone, Debug)]
pub enum VerifyEvent {
    Progress {
        done: usize,
        total: usize,
        current: String,
    },
    Record(VerifyRecord),
}

#[derive(Clone, Debug, Default)]
pub struct VerifyOutcome {
    pub records: Vec<VerifyRecord>,
    pub passed: usize,
    pub elapsed: Duration,
    pub stopped: bool,
}

struct WsConn {
    conn: http::Conn,
    buffer: Vec<u8>,
}

impl WsConn {
    fn new(conn: http::Conn, leftover: Vec<u8>) -> Self {
        Self {
            conn,
            buffer: leftover,
        }
    }

    async fn fill(&mut self, needed: usize, wait: Duration) -> Result<()> {
        while self.buffer.len() < needed {
            let mut chunk = [0u8; 16384];
            let read = timeout(wait, self.conn.read(&mut chunk))
                .await
                .map_err(|_| anyhow!("读取 WebSocket 帧超时"))?
                .context("读取 WebSocket 帧失败")?;
            if read == 0 {
                bail!("隧道被对端关闭");
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
        Ok(())
    }

    async fn take(&mut self, len: usize, wait: Duration) -> Result<Vec<u8>> {
        self.fill(len, wait).await?;
        Ok(self.buffer.drain(..len).collect())
    }

    async fn send_frame(&mut self, opcode: u8, payload: &[u8], wait: Duration) -> Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mut mask = [0u8; 4];
        rand::rng().fill_bytes(&mut mask);
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(ix, byte)| byte ^ mask[ix % 4]),
        );
        timeout(wait, self.conn.write_all(&frame))
            .await
            .map_err(|_| anyhow!("发送 WebSocket 帧超时"))?
            .context("发送 WebSocket 帧失败")
    }

    async fn read_frame(&mut self, wait: Duration) -> Result<(u8, Vec<u8>)> {
        loop {
            let header = self.take(2, wait).await?;
            let opcode = header[0] & 0x0f;
            let masked = header[1] & 0x80 != 0;
            let mut length = u64::from(header[1] & 0x7f);
            if length == 126 {
                let ext = self.take(2, wait).await?;
                length = u64::from(u16::from_be_bytes([ext[0], ext[1]]));
            } else if length == 127 {
                let ext = self.take(8, wait).await?;
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&ext);
                length = u64::from_be_bytes(bytes);
            }
            if length > 4 * 1024 * 1024 {
                bail!("WebSocket 帧过大：{length} 字节");
            }
            let mask = if masked {
                Some(self.take(4, wait).await?)
            } else {
                None
            };
            let mut payload = self.take(length as usize, wait).await?;
            if let Some(mask) = mask {
                for (ix, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[ix % 4];
                }
            }
            match opcode {
                0x9 => {
                    self.send_frame(0xA, &payload, wait).await?;
                    continue;
                }
                0xA => continue,
                0x8 => bail!("服务端关闭了隧道"),
                _ => return Ok((opcode, payload)),
            }
        }
    }
}

fn vless_header(uuid: &str, host: &str, port: u16) -> Result<Vec<u8>> {
    let uuid = Uuid::parse_str(uuid.trim()).context("UUID 格式无效")?;
    let host = host.as_bytes();
    if host.is_empty() || host.len() > 255 {
        bail!("目标主机名长度无效");
    }
    let mut header = Vec::with_capacity(1 + 16 + 2 + 2 + 2 + host.len());
    header.push(0); // version
    header.extend_from_slice(uuid.as_bytes());
    header.push(0); // addon length
    header.push(1); // command: TCP
    header.extend_from_slice(&port.to_be_bytes());
    header.push(2); // address type: domain
    header.push(host.len() as u8);
    header.extend_from_slice(host);
    Ok(header)
}

fn colo_from_head(head: &http::HttpHead) -> Option<String> {
    let value = head.header("cf-ray")?;
    let colo = value.rsplit('-').next()?.trim();
    if colo.is_empty() {
        None
    } else {
        Some(colo.to_ascii_uppercase())
    }
}

fn shorten(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

async fn verify_ws(target: &VerifyTarget, wait: Duration) -> VerifyRecord {
    let started = Instant::now();
    let mut base = VerifyRecord {
        ip: target.addr.clone(),
        port: target.port,
        transport: target.node.transport.clone(),
        ok: false,
        stage: String::new(),
        detail: String::new(),
        colo: None,
        ms: None,
    };

    let result = async {
        let sni = if target.node.sni.trim().is_empty() {
            target.node.addr.as_str()
        } else {
            target.node.sni.as_str()
        };
        let host = if target.node.host.trim().is_empty() {
            target.node.addr.as_str()
        } else {
            target.node.host.as_str()
        };
        let path = if target.node.path.starts_with('/') {
            target.node.path.clone()
        } else {
            format!("/{}", target.node.path)
        };

        let mut conn = http::connect_tls(&target.addr, target.port, sni, true, wait).await?;
        let key = http::base64_key();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: Mozilla/5.0\r\n\r\n"
        );
        http::send_all(&mut conn, request.as_bytes(), wait).await?;
        let head = http::read_head(&mut conn, 16 * 1024, wait).await?;
        if head.status != 101 {
            return Ok::<VerifyRecord, anyhow::Error>(VerifyRecord {
                stage: "WS升级".into(),
                detail: format!("HTTP {}", head.status),
                colo: colo_from_head(&head),
                ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                ..base.clone()
            });
        }

        let colo = colo_from_head(&head);
        let mut ws = WsConn::new(conn, head.leftover.clone());
        ws.send_frame(0x2, &vless_header(&target.node.uuid, PROBE_HOST, 80)?, wait)
            .await?;
        let inner = format!(
            "GET {PROBE_PATH} HTTP/1.1\r\nHost: {PROBE_HOST}\r\nUser-Agent: curl/8\r\nConnection: close\r\n\r\n"
        );
        ws.send_frame(0x2, inner.as_bytes(), wait).await?;

        let deadline = Instant::now() + wait;
        let mut received = Vec::new();
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (opcode, payload) = ws.read_frame(remaining.max(Duration::from_millis(100))).await?;
            if opcode == 0x8 {
                return Ok::<VerifyRecord, anyhow::Error>(VerifyRecord {
                    stage: "隧道".into(),
                    detail: "服务端关闭了隧道".into(),
                    colo,
                    ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                    ..base.clone()
                });
            }
            received.extend_from_slice(&payload);
            let head_zone = &received[..received.len().min(400)];
            if head_zone.windows(7).any(|window| window == b"HTTP/1.") {
                let text = String::from_utf8_lossy(head_zone);
                let first = text.lines().next().unwrap_or("").trim().to_string();
                let ok = first.contains(" 204") || first.contains(" 200");
                return Ok::<VerifyRecord, anyhow::Error>(VerifyRecord {
                    ok,
                    stage: if ok { "OK".into() } else { "回程".into() },
                    detail: shorten(&first, 80),
                    colo,
                    ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                    ..base.clone()
                });
            }
        }
        Ok::<VerifyRecord, anyhow::Error>(VerifyRecord {
            stage: "回程".into(),
            detail: if received.is_empty() {
                "隧道无回程数据".into()
            } else {
                shorten(&String::from_utf8_lossy(&received), 80)
            },
            colo,
            ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            ..base.clone()
        })
    }
    .await;

    match result {
        Ok(record) => record,
        Err(err) => {
            base.stage = "ERR".into();
            base.detail = shorten(&format!("{err:#}"), 90);
            base.ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            base
        }
    }
}

async fn verify_tls_only(target: &VerifyTarget, wait: Duration) -> VerifyRecord {
    let started = Instant::now();
    let mut record = VerifyRecord {
        ip: target.addr.clone(),
        port: target.port,
        transport: target.node.transport.clone(),
        ok: false,
        stage: "TLS+回源".into(),
        detail: String::new(),
        colo: None,
        ms: None,
    };
    let result = async {
        let sni = if target.node.sni.trim().is_empty() {
            target.node.addr.as_str()
        } else {
            target.node.sni.as_str()
        };
        let path = if target.node.path.starts_with('/') {
            target.node.path.clone()
        } else {
            format!("/{}", target.node.path)
        };
        let mut conn = http::connect_tls(&target.addr, target.port, sni, true, wait).await?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {sni}\r\nUser-Agent: Mozilla/5.0\r\nConnection: close\r\n\r\n"
        );
        http::send_all(&mut conn, request.as_bytes(), wait).await?;
        let head = http::read_head(&mut conn, 16 * 1024, wait).await?;
        let ok = matches!(head.status, 200 | 400 | 404);
        Ok::<VerifyRecord, anyhow::Error>(VerifyRecord {
            ok,
            detail: format!("[基础验证] HTTP {}", head.status),
            colo: colo_from_head(&head),
            ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            ..record.clone()
        })
    }
    .await;

    match result {
        Ok(done) => done,
        Err(err) => {
            record.ok = false;
            record.detail = shorten(&format!("{err:#}"), 90);
            record.ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            record
        }
    }
}

/// 内置的 TLS 弱验证节点，无需 UUID。
pub fn tls_probe_node() -> VlessNode {
    VlessNode {
        name: "TLS 基础验证".into(),
        uuid: String::new(),
        addr: PROBE_HOST.into(),
        port: 443,
        transport: "tls".into(),
        path: "/cdn-cgi/trace".into(),
        host: PROBE_HOST.into(),
        sni: PROBE_HOST.into(),
        params: Vec::new(),
    }
}

/// 选择验证任意 Cloudflare IP 时使用的节点：有 VLESS WebSocket 节点就走隧道验证；
/// 否则改用内置探测节点。其他传输（TCP / Reality / gRPC 等）的 SNI 未必托管在
/// Cloudflare，拿它做弱验证可能让所有 IP 都握手失败。
pub fn node_for_ips(nodes: &[VlessNode]) -> VlessNode {
    nodes
        .iter()
        .find(|node| node.transport.eq_ignore_ascii_case("ws"))
        .cloned()
        .unwrap_or_else(tls_probe_node)
}

pub async fn verify_one(target: &VerifyTarget, wait: Duration) -> VerifyRecord {
    if target.node.transport.eq_ignore_ascii_case("ws") {
        verify_ws(target, wait).await
    } else {
        verify_tls_only(target, wait).await
    }
}

pub async fn execute<F>(
    targets: &[VerifyTarget],
    wait: Duration,
    cancel: Arc<AtomicBool>,
    mut emit: F,
) -> VerifyOutcome
where
    F: FnMut(VerifyEvent) + Send,
{
    let started = Instant::now();
    let mut outcome = VerifyOutcome::default();
    for (ix, target) in targets.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcome.stopped = true;
            break;
        }
        let label = format!("{}:{}", target.addr, target.port);
        emit(VerifyEvent::Progress {
            done: ix,
            total: targets.len(),
            current: label.clone(),
        });
        let record = timeout(wait + Duration::from_secs(4), verify_one(target, wait))
            .await
            .unwrap_or_else(|_| VerifyRecord {
                ip: target.addr.clone(),
                port: target.port,
                transport: target.node.transport.clone(),
                ok: false,
                stage: "ERR".into(),
                detail: "验证超时".into(),
                colo: None,
                ms: None,
            });
        if record.ok {
            outcome.passed += 1;
        }
        emit(VerifyEvent::Record(record.clone()));
        outcome.records.push(record);
        emit(VerifyEvent::Progress {
            done: ix + 1,
            total: targets.len(),
            current: label,
        });
    }
    outcome.elapsed = started.elapsed();
    if cancel.load(Ordering::Relaxed) {
        outcome.stopped = true;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_vless_header() {
        let header = vless_header(
            "b831381d-6324-4d53-ad4f-8cda48b30811",
            "cp.cloudflare.com",
            80,
        )
        .unwrap();
        assert_eq!(header[0], 0);
        assert_eq!(header[17], 0);
        assert_eq!(header[18], 1);
        assert_eq!(&header[19..21], &80u16.to_be_bytes());
        assert_eq!(header[21], 2);
        assert_eq!(header[22] as usize, "cp.cloudflare.com".len());
    }

    #[test]
    fn ip_verification_prefers_websocket_and_falls_back_to_probe() {
        let node = |name: &str, transport: &str| VlessNode {
            name: name.into(),
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            addr: "example.com".into(),
            port: 443,
            transport: transport.into(),
            path: "/".into(),
            host: "www.microsoft.com".into(),
            sni: "www.microsoft.com".into(),
            params: Vec::new(),
        };

        for nodes in [vec![], vec![node("reality", "tcp"), node("grpc", "grpc")]] {
            let picked = node_for_ips(&nodes);
            assert_eq!(picked.transport, "tls");
            assert!(picked.uuid.is_empty());
            assert_eq!(picked.sni, PROBE_HOST);
            assert_eq!(picked.host, PROBE_HOST);
        }

        let picked = node_for_ips(&[node("reality", "tcp"), node("cf-ws", "WS")]);
        assert_eq!(picked.name, "cf-ws");
        assert_eq!(picked.sni, "www.microsoft.com");
    }
}
