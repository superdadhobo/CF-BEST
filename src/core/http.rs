use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use yaml_rust2::{Yaml, YamlLoader};

use crate::core::model::VlessNode;
use crate::core::{ip, storage};

pub type Conn = TlsStream<TcpStream>;

pub const TRACE_SNI: &str = "cloudflare.com";
pub const SPEED_SNI: &str = "speed.cloudflare.com";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// 连接裸 IP 时证书主机名必然不匹配，扫描 / 测速 / 验证只需要握手成功：
/// 接受任意证书，但仍校验握手签名（与原 native-tls 的 `danger_accept_invalid_*` 等价）。
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// 只说 HTTP/1.1（含 WebSocket 升级），显式声明 ALPN 以免服务器选择 h2。
fn finish(mut config: ClientConfig) -> TlsConnector {
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsConnector::from(Arc::new(config))
}

fn insecure_connector() -> Result<&'static TlsConnector> {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    if let Some(connector) = CONNECTOR.get() {
        return Ok(connector);
    }
    let provider = provider();
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .context("初始化 TLS 连接器失败")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
        .with_no_client_auth();
    Ok(CONNECTOR.get_or_init(|| finish(config)))
}

/// 校验证书的连接器：信任 Windows 证书存储（与原 SChannel 行为一致）。
fn verified_connector() -> Result<&'static TlsConnector> {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    if let Some(connector) = CONNECTOR.get() {
        return Ok(connector);
    }
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for error in &native.errors {
        storage::log_line(&format!("读取系统根证书时出错：{error}"));
    }
    let (added, _ignored) = roots.add_parsable_certificates(native.certs);
    if added == 0 {
        bail!("没有可用的系统根证书");
    }
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .context("初始化 TLS 连接器失败")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(CONNECTOR.get_or_init(|| finish(config)))
}

async fn resolve(host: &str, port: u16, wait: Duration) -> Result<SocketAddr> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    let addrs = timeout(wait, lookup_host((host, port)))
        .await
        .map_err(|_| anyhow!("DNS 解析超时：{host}"))?
        .with_context(|| format!("DNS 解析失败：{host}"))?;
    let addrs: Vec<SocketAddr> = addrs.collect();
    addrs
        .iter()
        .find(|addr| addr.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| anyhow!("DNS 无结果：{host}"))
}

pub async fn connect_tls(
    host: &str,
    port: u16,
    sni: &str,
    insecure: bool,
    wait: Duration,
) -> Result<Conn> {
    let addr = resolve(host, port, wait).await?;
    let tcp = timeout(wait, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("TCP 连接超时：{host}:{port}"))?
        .with_context(|| format!("TCP 连接失败：{host}:{port}"))?;
    let _ = tcp.set_nodelay(true);
    let connector = if insecure {
        insecure_connector()?
    } else {
        verified_connector()?
    };
    let sni = if sni.trim().is_empty() { host } else { sni };
    let server_name =
        ServerName::try_from(sni.to_string()).map_err(|_| anyhow!("无效的 SNI：{sni}"))?;
    timeout(wait, connector.connect(server_name, tcp))
        .await
        .map_err(|_| anyhow!("TLS 握手超时：{host}:{port}"))?
        .with_context(|| format!("TLS 握手失败：{host}:{port} (SNI {sni})"))
}

pub async fn send_all(stream: &mut Conn, bytes: &[u8], wait: Duration) -> Result<()> {
    timeout(wait, stream.write_all(bytes))
        .await
        .map_err(|_| anyhow!("发送请求超时"))?
        .context("发送请求失败")
}

#[derive(Clone, Debug, Default)]
pub struct HttpHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub leftover: Vec<u8>,
}

impl HttpHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_head(bytes: &[u8]) -> Result<HttpHead> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("无效的 HTTP 状态行：{status_line}"))?;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok(HttpHead {
        status,
        headers,
        leftover: Vec::new(),
    })
}

pub async fn read_head(stream: &mut Conn, max_head: usize, wait: Duration) -> Result<HttpHead> {
    let future = async {
        let mut buffer = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(pos) = find_head_end(&buffer) {
                let mut head = parse_head(&buffer[..pos])?;
                head.leftover = buffer[pos + 4..].to_vec();
                return Ok(head);
            }
            if buffer.len() >= max_head {
                bail!("响应头超过 {max_head} 字节");
            }
            let read = stream.read(&mut chunk).await.context("读取响应头失败")?;
            if read == 0 {
                if buffer.is_empty() {
                    bail!("连接被对端关闭");
                }
                bail!("响应头不完整");
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
    };
    timeout(wait, future)
        .await
        .map_err(|_| anyhow!("读取响应头超时"))?
}

pub async fn read_to_end_limited<S: AsyncRead + Unpin>(
    stream: &mut S,
    initial: Vec<u8>,
    max: usize,
    wait: Duration,
) -> Result<Vec<u8>> {
    let future = async {
        let mut buffer = initial;
        let mut chunk = [0u8; 16384];
        while buffer.len() < max {
            let read = stream.read(&mut chunk).await.context("读取响应体失败")?;
            if read == 0 {
                break;
            }
            let remaining = max - buffer.len();
            buffer.extend_from_slice(&chunk[..read.min(remaining)]);
        }
        Ok(buffer)
    };
    timeout(wait, future)
        .await
        .map_err(|_| anyhow!("读取响应体超时"))?
}

pub fn decode_chunked(data: &[u8]) -> Result<Vec<u8>> {
    let mut cursor = 0usize;
    let mut out = Vec::with_capacity(data.len());
    while let Some(line_end) = data[cursor..]
        .windows(2)
        .position(|window| window == b"\r\n")
    {
        let line = std::str::from_utf8(&data[cursor..cursor + line_end])
            .context("chunk 头不是有效 UTF-8")?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .with_context(|| format!("无效的 chunk 大小：{size_text}"))?;
        cursor += line_end + 2;
        if size == 0 {
            break;
        }
        if cursor + size > data.len() {
            bail!("chunk 数据不完整");
        }
        out.extend_from_slice(&data[cursor..cursor + size]);
        cursor += size;
        if data.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else {
            break;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
struct UrlParts {
    host: String,
    port: u16,
    path: String,
}

fn parse_https_url(url: &str) -> Result<UrlParts> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .ok_or_else(|| anyhow!("仅支持 https:// 地址"))?;
    let (authority, path) = match rest.find('/') {
        Some(ix) => (&rest[..ix], &rest[ix..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, tail) = stripped
            .split_once(']')
            .ok_or_else(|| anyhow!("IPv6 地址格式无效"))?;
        let port = tail
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(443);
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .with_context(|| format!("URL 端口无效：{port}"))?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 443)
    };
    if host.is_empty() {
        bail!("URL 缺少主机名");
    }
    Ok(UrlParts {
        host,
        port,
        path: path.to_string(),
    })
}

fn resolve_location(base: &UrlParts, location: &str) -> Result<UrlParts> {
    let location = location.split('#').next().unwrap_or(location);
    if location.starts_with("https://") {
        return parse_https_url(location);
    }
    if location.starts_with("http://") {
        bail!("服务端重定向到不安全的 http:// 地址");
    }
    if location.starts_with('/') {
        return Ok(UrlParts {
            host: base.host.clone(),
            port: base.port,
            path: location.to_string(),
        });
    }
    let parent = match base.path.rfind('/') {
        Some(ix) => &base.path[..=ix],
        None => "/",
    };
    Ok(UrlParts {
        host: base.host.clone(),
        port: base.port,
        path: format!("{parent}{location}"),
    })
}

pub async fn https_get(url: &str, insecure: bool, wait: Duration) -> Result<Vec<u8>> {
    let (mut conn, head) = open_get(url, insecure, wait).await?;
    let body = read_to_end_limited(&mut conn, head.leftover.clone(), 8 * 1024 * 1024, wait).await?;
    decode_body(&head, body)
}

/// 发出 GET 并跟随重定向（最多 5 次），返回 2xx 响应的连接与响应头；正文由调用方读取，
/// 已读到的部分在 `head.leftover` 里。
pub async fn open_get(url: &str, insecure: bool, wait: Duration) -> Result<(Conn, HttpHead)> {
    let mut target = parse_https_url(url)?;
    for _ in 0..5 {
        let mut conn = connect_tls(&target.host, target.port, &target.host, insecure, wait).await?;
        let host_header = if target.port == 443 {
            target.host.clone()
        } else {
            format!("{}:{}", target.host, target.port)
        };
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Mozilla/5.0\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
            target.path, host_header
        );
        send_all(&mut conn, request.as_bytes(), wait).await?;
        let head = read_head(&mut conn, 64 * 1024, wait).await?;
        if (300..400).contains(&head.status)
            && let Some(location) = head.header("location")
        {
            target = resolve_location(&target, location)?;
            continue;
        }
        if !(200..300).contains(&head.status) {
            bail!("HTTP {}（{}）", head.status, target.host);
        }
        return Ok((conn, head));
    }
    bail!("重定向次数过多")
}

/// 按响应头还原正文：chunked 解码，或按 Content-Length 截掉多余的字节。
pub fn decode_body(head: &HttpHead, body: Vec<u8>) -> Result<Vec<u8>> {
    let chunked = head
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    if chunked {
        decode_chunked(&body)
    } else if let Some(length) = head
        .header("content-length")
        .and_then(|value| value.parse::<usize>().ok())
    {
        Ok(body[..body.len().min(length)].to_vec())
    } else {
        Ok(body)
    }
}

pub async fn probe_trace(ip: &str, port: u16, wait: Duration) -> Result<(String, f64)> {
    let started = Instant::now();
    let future = async {
        let mut conn = connect_tls(ip, port, TRACE_SNI, true, wait).await?;
        let request = format!(
            "GET /cdn-cgi/trace HTTP/1.1\r\nHost: {TRACE_SNI}\r\nUser-Agent: ipscan\r\nConnection: close\r\n\r\n"
        );
        send_all(&mut conn, request.as_bytes(), wait).await?;
        let head = read_head(&mut conn, 16 * 1024, wait).await?;
        if !(200..300).contains(&head.status) {
            bail!("HTTP {}", head.status);
        }
        let body = read_to_end_limited(&mut conn, head.leftover.clone(), 8192, wait).await?;
        let text = String::from_utf8_lossy(&body);
        let colo = text
            .lines()
            .find_map(|line| line.strip_prefix("colo="))
            .map(|colo| colo.trim().to_ascii_uppercase())
            .filter(|colo| !colo.is_empty())
            .ok_or_else(|| anyhow!("响应中没有 colo"))?;
        Ok(colo)
    };
    let colo = timeout(wait, future)
        .await
        .map_err(|_| anyhow!("trace 超时"))??;
    Ok((colo, started.elapsed().as_secs_f64() * 1000.0))
}

pub async fn fetch_official_cidrs(wait: Duration) -> (Vec<String>, String) {
    let urls = [
        ("官方 API", "https://api.cloudflare.com/client/v4/ips"),
        ("官方文本", "https://www.cloudflare.com/ips-v4"),
    ];
    for (source, url) in urls {
        match https_get(url, false, wait).await {
            Ok(body) => {
                let text = String::from_utf8_lossy(&body);
                let mut cidrs = Vec::new();
                if text.trim_start().starts_with('{') {
                    if let Ok(json) = serde_json::from_str::<Value>(&text)
                        && let Some(list) = json["result"]["ipv4_cidrs"].as_array()
                    {
                        for item in list {
                            if let Some(cidr) = item.as_str() {
                                cidrs.push(cidr.to_string());
                            }
                        }
                    }
                } else {
                    for line in text.lines() {
                        let line = line.trim();
                        if line.contains('/') && !line.contains(':') {
                            cidrs.push(line.to_string());
                        }
                    }
                }
                if !cidrs.is_empty() {
                    return (cidrs, source.to_string());
                }
            }
            Err(err) => storage::log_line(&format!("{source} 拉取失败：{err:#}")),
        }
    }
    (
        ip::FALLBACK_RANGES
            .iter()
            .map(|cidr| cidr.to_string())
            .collect(),
        "内置兜底".to_string(),
    )
}

pub async fn fetch_asn_prefixes(asn_list: &str, wait: Duration) -> Result<(Vec<String>, String)> {
    let mut prefixes = Vec::new();
    let mut sources = Vec::new();
    for asn in asn_list
        .split(',')
        .map(str::trim)
        .filter(|asn| !asn.is_empty())
    {
        let normalized = if asn.to_ascii_uppercase().starts_with("AS") {
            asn.to_ascii_uppercase()
        } else {
            format!("AS{asn}")
        };
        let url = format!(
            "https://stat.ripe.net/data/announced-prefixes/data.json?resource={normalized}"
        );
        let body = https_get(&url, false, wait)
            .await
            .with_context(|| format!("{normalized} 前缀拉取失败"))?;
        let json: Value =
            serde_json::from_slice(&body).with_context(|| format!("{normalized} 响应解析失败"))?;
        let mut count = 0;
        if let Some(list) = json["data"]["prefixes"].as_array() {
            for item in list {
                if let Some(prefix) = item["prefix"].as_str()
                    && !prefix.contains(':')
                {
                    prefixes.push(prefix.to_string());
                    count += 1;
                }
            }
        }
        if count == 0 {
            bail!("{normalized} 没有返回 IPv4 前缀");
        }
        sources.push(format!("{normalized} · {count} 个 IPv4 前缀"));
    }
    prefixes.sort();
    prefixes.dedup();
    if prefixes.is_empty() {
        bail!("没有获取到任何前缀");
    }
    Ok((prefixes, sources.join("；")))
}

/// 订阅拉取失败后的重试间隔：共尝试 3 次。订阅地址在部分网络下时通时断
/// （TLS 握手超时、读取响应超时），隔几秒重试往往就能成功。
const SUBSCRIPTION_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];

/// 拉取并解析订阅。网络请求失败（超时、连接中断、HTTP 错误）按 [`SUBSCRIPTION_RETRY_DELAYS`]
/// 重试；拿到内容但解析不出节点时不重试，再拉一次结果也一样。
pub async fn fetch_subscription(url: &str, wait: Duration) -> Result<Vec<VlessNode>> {
    let (body, attempts) = with_retries(
        &SUBSCRIPTION_RETRY_DELAYS,
        |attempt, err, delay| {
            storage::log_line(&format!(
                "订阅拉取失败（第 {attempt} 次），{} 秒后重试：{err:#}",
                delay.as_secs()
            ));
        },
        || https_get(url, false, wait),
    )
    .await;
    let body = body.map_err(|err| {
        if attempts > 1 {
            anyhow!("{err:#}（共尝试 {attempts} 次）")
        } else {
            err
        }
    })?;
    parse_subscription(&String::from_utf8_lossy(&body))
}

/// 依次尝试，第 n 次失败后等待 `delays[n-1]` 再试，`delays` 用完即放弃；返回结果与尝试次数。
/// `on_retry` 收到失败的次序、错误与随后的等待时长。
async fn with_retries<T, Fut>(
    delays: &[Duration],
    mut on_retry: impl FnMut(usize, &anyhow::Error, Duration),
    mut attempt: impl FnMut() -> Fut,
) -> (Result<T>, usize)
where
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempts = 0;
    loop {
        attempts += 1;
        match attempt().await {
            Ok(value) => return (Ok(value), attempts),
            Err(err) => match delays.get(attempts - 1) {
                Some(&delay) => {
                    on_retry(attempts, &err, delay);
                    tokio::time::sleep(delay).await;
                }
                None => return (Err(err), attempts),
            },
        }
    }
}

/// 订阅内容：vless:// 分享链接（明文或 Base64），或 Clash / mihomo 配置里的 VLESS 节点。
pub fn parse_subscription(text: &str) -> Result<Vec<VlessNode>> {
    let text = text.trim();
    let links = if text.contains("vless://") {
        text.to_string()
    } else {
        decode_base64_text(text).unwrap_or_else(|| text.to_string())
    };
    let mut nodes = Vec::new();
    for line in links.lines() {
        let line = line.trim();
        if line.starts_with("vless://")
            && let Some(node) = parse_vless_link(line)
        {
            nodes.push(node);
        }
    }
    if nodes.is_empty() {
        nodes = parse_clash_proxies(text);
    }
    if nodes.is_empty() {
        bail!("订阅中没有找到 VLESS 节点（支持 vless:// 分享链接与 Clash 配置）");
    }
    Ok(nodes)
}

/// Clash / mihomo 配置 `proxies` 列表中的 VLESS 节点，换算成分享链接的字段。默认值与 mihomo 一致：
/// Host 头缺省取服务器地址，SNI 依次取 servername、Host 头、服务器地址；
/// `max-early-data` 写成路径里的 `ed` 参数，与分享链接的约定相同。
fn parse_clash_proxies(text: &str) -> Vec<VlessNode> {
    let Ok(docs) = YamlLoader::load_from_str(text) else {
        return Vec::new();
    };
    let Some(proxies) = docs.first().and_then(|doc| doc["proxies"].as_vec()) else {
        return Vec::new();
    };
    proxies.iter().filter_map(clash_vless).collect()
}

fn clash_vless(proxy: &Yaml) -> Option<VlessNode> {
    if !yaml_text(&proxy["type"])?.eq_ignore_ascii_case("vless") {
        return None;
    }
    let addr = yaml_text(&proxy["server"])?;
    let uuid = yaml_text(&proxy["uuid"])?;
    let port = yaml_port(&proxy["port"]).unwrap_or(443);
    let transport = yaml_text(&proxy["network"])
        .unwrap_or_else(|| "tcp".into())
        .to_ascii_lowercase();
    let tls = yaml_flag(&proxy["tls"]);
    let ws = &proxy["ws-opts"];
    let host_header = ws["headers"].as_hash().and_then(|headers| {
        headers.iter().find_map(|(key, value)| {
            key.as_str()
                .is_some_and(|key| key.eq_ignore_ascii_case("host"))
                .then(|| yaml_text(value))
                .flatten()
        })
    });
    let host = host_header.clone().unwrap_or_else(|| addr.clone());
    let sni = yaml_text(&proxy["servername"])
        .or_else(|| yaml_text(&proxy["sni"]))
        .or(host_header)
        .unwrap_or_else(|| addr.clone());
    let mut path = yaml_text(&ws["path"]).unwrap_or_else(|| "/".into());
    if let Some(early) = yaml_port(&ws["max-early-data"]).filter(|early| *early > 0)
        && !path.contains("ed=")
    {
        let joiner = if path.contains('?') { '&' } else { '?' };
        path = format!("{path}{joiner}ed={early}");
    }

    let mut params = vec![
        ("encryption".to_string(), "none".to_string()),
        (
            "security".to_string(),
            if tls { "tls" } else { "none" }.to_string(),
        ),
    ];
    if tls {
        params.push(("sni".into(), sni.clone()));
    }
    if let Some(fingerprint) = yaml_text(&proxy["client-fingerprint"]) {
        params.push(("fp".into(), fingerprint));
    }
    if let Some(alpn) = proxy["alpn"].as_vec() {
        let alpn: Vec<String> = alpn.iter().filter_map(yaml_text).collect();
        if !alpn.is_empty() {
            params.push(("alpn".into(), alpn.join(",")));
        }
    }
    if yaml_flag(&proxy["skip-cert-verify"]) {
        params.push(("allowInsecure".into(), "1".into()));
    }
    params.push(("type".into(), transport.clone()));
    params.push(("host".into(), host.clone()));
    params.push(("path".into(), path.clone()));

    Some(VlessNode {
        name: yaml_text(&proxy["name"]).unwrap_or_default(),
        uuid,
        addr,
        port,
        transport,
        path,
        host,
        sni,
        params,
    })
}

fn yaml_text(value: &Yaml) -> Option<String> {
    let text = match value {
        Yaml::String(text) | Yaml::Real(text) => text.trim().to_string(),
        Yaml::Integer(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn yaml_port(value: &Yaml) -> Option<u16> {
    match value {
        Yaml::Integer(number) => u16::try_from(*number).ok(),
        Yaml::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn yaml_flag(value: &Yaml) -> bool {
    match value {
        Yaml::Boolean(flag) => *flag,
        Yaml::String(text) => text.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn decode_base64_text(text: &str) -> Option<String> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let padded = format!("{cleaned}{}", "=".repeat((4 - cleaned.len() % 4) % 4));
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(padded.as_bytes())
                .ok()
        })
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b'%' if ix + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[ix + 1..ix + 3]).ok();
                if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    out.push(byte);
                    ix += 3;
                    continue;
                }
                out.push(bytes[ix]);
                ix += 1;
            }
            b'+' => {
                out.push(b' ');
                ix += 1;
            }
            byte => {
                out.push(byte);
                ix += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_vless_link(link: &str) -> Option<VlessNode> {
    let rest = link.trim().strip_prefix("vless://")?;
    let (rest, name) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, percent_decode(fragment)),
        None => (rest, String::new()),
    };
    let (authority, query) = match rest.split_once('?') {
        Some((authority, query)) => (authority, query),
        None => (rest, ""),
    };
    let (uuid, host_port) = authority.split_once('@')?;
    let (addr, port) = if let Some(stripped) = host_port.strip_prefix('[') {
        let (host, tail) = stripped.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(443);
        (host.to_string(), port)
    } else if let Some((host, port)) = host_port.rsplit_once(':') {
        (host.to_string(), port.parse::<u16>().unwrap_or(443))
    } else {
        (host_port.to_string(), 443)
    };
    if addr.is_empty() || uuid.is_empty() {
        return None;
    }
    let params: Vec<(String, String)> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect();
    // 同名参数以最后一个为准；空值视为未填写。
    let param = |name: &str| {
        params
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    };
    let host = param("host").unwrap_or_else(|| addr.clone());
    let sni = param("sni").unwrap_or_else(|| addr.clone());
    let path = param("path").unwrap_or_else(|| "/".to_string());
    let transport = param("type")
        .unwrap_or_else(|| "tcp".to_string())
        .to_ascii_lowercase();
    Some(VlessNode {
        name,
        uuid: uuid.to_string(),
        addr,
        port,
        transport,
        path,
        host,
        sni,
        params,
    })
}

pub fn base64_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url() {
        let url = parse_https_url("https://stat.ripe.net/data/x.json?resource=AS906").unwrap();
        assert_eq!(url.host, "stat.ripe.net");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "/data/x.json?resource=AS906");
    }

    #[test]
    fn decodes_chunked_body() {
        let data = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(data).unwrap(), b"Wikipedia");
    }

    #[test]
    fn parses_vless_ws() {
        let link = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@example.com:443?encryption=none&security=tls&sni=edge.example.com&type=ws&host=edge.example.com&path=%2Fw-123#HK%20node";
        let node = parse_vless_link(link).unwrap();
        assert_eq!(node.uuid, "b831381d-6324-4d53-ad4f-8cda48b30811");
        assert_eq!(node.addr, "example.com");
        assert_eq!(node.sni, "edge.example.com");
        assert_eq!(node.path, "/w-123");
        assert_eq!(node.transport, "ws");
        assert_eq!(node.name, "HK node");
        assert_eq!(node.params.len(), 6);
        assert_eq!(node.params[1], ("security".into(), "tls".into()));
        assert_eq!(node.params[5], ("path".into(), "/w-123".into()));
    }

    #[test]
    fn parses_block_style_clash_subscription() {
        // 与 edgetunnel 等面板输出的 Clash 配置同形：顶层 dns / proxies / proxy-groups / rules。
        let yaml = "dns:\n  enable: true\nproxies:\n- name: HK 01\n  type: vless\n  server: edge.example.com\n  port: 443\n  uuid: b831381d-6324-4d53-ad4f-8cda48b30811\n  network: ws\n  tls: true\n  servername: sni.example.com\n  client-fingerprint: chrome\n  udp: true\n  ws-opts:\n    path: /?ed=2560\n    headers:\n      Host: host.example.com\n- name: other\n  type: trojan\n  server: t.example.com\n  port: 443\n  password: secret\nproxy-groups:\n- name: auto\n  type: url-test\n  proxies:\n  - HK 01\nrules:\n- MATCH,auto\n";
        let nodes = parse_subscription(yaml).unwrap();
        assert_eq!(nodes.len(), 1);
        let node = &nodes[0];
        assert_eq!(node.name, "HK 01");
        assert_eq!(node.addr, "edge.example.com");
        assert_eq!(node.port, 443);
        assert_eq!(node.uuid, "b831381d-6324-4d53-ad4f-8cda48b30811");
        assert_eq!(node.transport, "ws");
        assert_eq!(node.path, "/?ed=2560");
        assert_eq!(node.host, "host.example.com");
        assert_eq!(node.sni, "sni.example.com");
        assert!(node.params.contains(&("fp".into(), "chrome".into())));
        assert!(node.params.contains(&("security".into(), "tls".into())));
        // 换算出的参数能拼回等价的分享链接。
        let query = node
            .params
            .iter()
            .map(|(key, value)| format!("{key}={}", crate::core::proxy::percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let link = parse_vless_link(&format!(
            "vless://{}@{}:{}?{query}#x",
            node.uuid, node.addr, node.port
        ))
        .unwrap();
        assert_eq!(
            (&link.host, &link.sni, &link.path, &link.transport),
            (&node.host, &node.sni, &node.path, &node.transport)
        );
    }

    #[test]
    fn parses_flow_style_clash_subscription_with_mihomo_defaults() {
        let yaml = "proxies:\n  - {name: \"US 1\", server: 104.16.1.2, port: 2053, type: vless, uuid: b831381d-6324-4d53-ad4f-8cda48b30811, tls: true, skip-cert-verify: true, alpn: [http/1.1], network: ws, ws-opts: {path: /ws, max-early-data: 2048, early-data-header-name: Sec-WebSocket-Protocol}}\n  - {name: plain, server: a.example.com, port: 80, type: vless, uuid: b831381d-6324-4d53-ad4f-8cda48b30811, network: ws, ws-opts: {path: \"/x?y=1\", max-early-data: 1024, headers: {host: h.example.com}}}\n";
        let nodes = parse_subscription(yaml).unwrap();
        assert_eq!(nodes.len(), 2);
        let first = &nodes[0];
        assert_eq!((first.addr.as_str(), first.port), ("104.16.1.2", 2053));
        assert_eq!(first.host, "104.16.1.2");
        assert_eq!(first.sni, "104.16.1.2");
        assert_eq!(first.path, "/ws?ed=2048");
        assert!(first.params.contains(&("allowInsecure".into(), "1".into())));
        assert!(first.params.contains(&("alpn".into(), "http/1.1".into())));
        let second = &nodes[1];
        assert_eq!(second.host, "h.example.com");
        assert_eq!(second.sni, "h.example.com");
        assert_eq!(second.path, "/x?y=1&ed=1024");
        assert!(second.params.contains(&("security".into(), "none".into())));
        assert!(!second.params.iter().any(|(key, _)| key == "sni"));
    }

    #[test]
    fn parses_link_subscriptions_and_rejects_empty_ones() {
        let link =
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@example.com:443?security=tls&type=ws#a";
        assert_eq!(parse_subscription(link).unwrap().len(), 1);
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{link}\nvmess://eyJ2IjoyfQ==\n"));
        assert_eq!(parse_subscription(&encoded).unwrap().len(), 1);
        assert!(parse_subscription("proxies: []\n").is_err());
        assert!(
            parse_subscription("proxies:\n- {name: t, type: trojan, server: a, port: 1}\n")
                .is_err()
        );
        assert!(parse_subscription("<html><body>login</body></html>").is_err());
        assert!(parse_subscription("").is_err());
    }

    #[tokio::test]
    async fn retries_until_success_or_delays_run_out() {
        let delays = [Duration::ZERO, Duration::ZERO];

        let mut calls = 0;
        let mut retried = Vec::new();
        let (result, attempts) = with_retries(
            &delays,
            |attempt, err, _| retried.push(format!("{attempt}:{err}")),
            || {
                calls += 1;
                let call = calls;
                async move {
                    if call < 3 {
                        bail!("超时 {call}");
                    }
                    Ok(call)
                }
            },
        )
        .await;
        assert_eq!(result.unwrap(), 3);
        assert_eq!(attempts, 3);
        assert_eq!(retried, vec!["1:超时 1", "2:超时 2"]);

        let mut calls = 0;
        let (result, attempts) = with_retries(
            &delays,
            |_, _, _| {},
            || {
                calls += 1;
                async { Err::<(), _>(anyhow!("连接被重置")) }
            },
        )
        .await;
        assert_eq!(result.unwrap_err().to_string(), "连接被重置");
        assert_eq!((attempts, calls), (3, 3));

        let (result, attempts) = with_retries(&[], |_, _, _| {}, || async { Ok(7) }).await;
        assert_eq!((result.unwrap(), attempts), (7, 1));
        let (result, attempts) =
            with_retries(&[], |_, _, _| {}, || async { Err::<(), _>(anyhow!("x")) }).await;
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    #[ignore = "requires network access to Cloudflare"]
    async fn insecure_tls_trace_reaches_cloudflare_edge() {
        // 裸 IP + SNI cloudflare.com：证书主机名不匹配，只有「接受任意证书」的连接器能握手。
        let (colo, ms) = probe_trace("104.16.132.229", 443, Duration::from_secs(15))
            .await
            .expect("trace via insecure rustls connector");
        assert!(colo.len() == 3, "unexpected colo {colo}");
        assert!(ms > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires network access to Cloudflare"]
    async fn verified_tls_rejects_hostname_mismatch_and_accepts_valid() {
        let wait = Duration::from_secs(15);
        let body = https_get("https://www.cloudflare.com/cdn-cgi/trace", false, wait)
            .await
            .expect("verified GET with system roots");
        assert!(String::from_utf8_lossy(&body).contains("colo="));
        // 校验连接器必须拒绝主机名不匹配的证书。
        let mismatch = connect_tls("104.16.132.229", 443, "example.invalid", false, wait).await;
        assert!(
            mismatch.is_err(),
            "verified connector accepted a mismatched certificate"
        );
    }

    #[tokio::test]
    #[ignore = "requires network access to stat.ripe.net"]
    async fn fetches_ripe_prefixes() {
        let (prefixes, source) = fetch_asn_prefixes("AS906", Duration::from_secs(30))
            .await
            .expect("AS906 prefixes");
        assert!(!prefixes.is_empty());
        assert!(source.contains("AS906"));
        assert!(prefixes.iter().all(|prefix| !prefix.contains(':')));
    }

    #[test]
    fn resolves_redirects() {
        let base = UrlParts {
            host: "a.com".into(),
            port: 443,
            path: "/api/v1/x".into(),
        };
        let next = resolve_location(&base, "../y").unwrap();
        assert_eq!(next.path, "/api/v1/../y");
        let next = resolve_location(&base, "/z").unwrap();
        assert_eq!(next.path, "/z");
    }
}
