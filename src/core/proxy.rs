//! 「导入代理软件」：把隧道验证通过的 IP 套到订阅节点上，生成代理软件能直接导入的节点。
//!
//! Cloudflare Workers / Pages 搭建的 VLESS + WS 节点只认 SNI 与 Host，入口地址换成任意
//! Cloudflare IP 都连到同一个节点；隧道验证通过的 IP 就是在本机网络下实测可用的入口。

use std::collections::HashSet;
use std::fmt::Write as _;

use base64::Engine as _;

use crate::core::model::VlessNode;

/// 验证通过、要写进节点的入口地址。
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub ip: String,
    pub port: u16,
    pub colo: String,
}

impl Endpoint {
    /// 节点名：机房 + 地址。不带排名和速度，订阅更新后同一 IP 的名字不变，客户端里选中的节点不会丢。
    pub fn label(&self) -> String {
        let address = if self.port == 443 {
            self.ip.clone()
        } else {
            format!("{}:{}", self.ip, self.port)
        };
        if self.colo.trim().is_empty() {
            address
        } else {
            format!("{} {address}", self.colo.trim())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedFormat {
    /// Clash / mihomo 配置（YAML）：Clash Verge、FlClash、Clash Party、Stash 等。
    Clash,
    /// Base64 编码的分享链接列表：v2rayN、Karing、Hiddify、Shadowrocket 等。
    V2ray,
}

impl FeedFormat {
    pub fn slug(self) -> &'static str {
        match self {
            FeedFormat::Clash => "clash",
            FeedFormat::V2ray => "v2ray",
        }
    }
}

/// 一组可导入的优选节点：订阅节点模板 × 验证通过的入口地址。
#[derive(Clone, Debug)]
pub struct ProxyFeed {
    /// 订阅名，例如「CF Best 自动优选」。
    pub title: String,
    /// 结果时间，写进配置注释。
    pub updated: String,
    pub template: VlessNode,
    pub endpoints: Vec<Endpoint>,
}

const GROUP_SELECT: &str = "CF Best";
const GROUP_AUTO: &str = "自动选择";
const TEST_URL: &str = "https://www.gstatic.com/generate_204";

/// 本机、局域网与中国大陆直连，其余走优选节点。私有网段不依赖 GeoIP 数据库。
const DIRECT_RULES: &[&str] = &[
    "DOMAIN-SUFFIX,local,DIRECT",
    "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
    "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
    "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
    "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
    "IP-CIDR,169.254.0.0/16,DIRECT,no-resolve",
    "IP-CIDR,100.64.0.0/10,DIRECT,no-resolve",
    "IP-CIDR6,::1/128,DIRECT,no-resolve",
    "IP-CIDR6,fc00::/7,DIRECT,no-resolve",
    "IP-CIDR6,fe80::/10,DIRECT,no-resolve",
    "DOMAIN-SUFFIX,cn,DIRECT",
    "GEOIP,CN,DIRECT",
];

impl ProxyFeed {
    /// 只接受带 UUID 的 VLESS WebSocket 节点（隧道验证用的就是它）；入口按 IP:端口去重，
    /// 一个都没有时返回 None。
    pub fn new(
        title: impl Into<String>,
        updated: impl Into<String>,
        template: VlessNode,
        endpoints: impl IntoIterator<Item = Endpoint>,
    ) -> Option<Self> {
        if !template.transport.eq_ignore_ascii_case("ws") || template.uuid.trim().is_empty() {
            return None;
        }
        let mut seen = HashSet::new();
        let endpoints: Vec<Endpoint> = endpoints
            .into_iter()
            .filter(|endpoint| seen.insert((endpoint.ip.clone(), endpoint.port)))
            .collect();
        if endpoints.is_empty() {
            return None;
        }
        Some(Self {
            title: title.into(),
            updated: updated.into(),
            template,
            endpoints,
        })
    }

    pub fn share_links(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .map(|endpoint| share_link(&self.template, endpoint))
            .collect()
    }

    pub fn render(&self, format: FeedFormat) -> String {
        match format {
            FeedFormat::Clash => self.clash_config(),
            FeedFormat::V2ray => {
                base64::engine::general_purpose::STANDARD.encode(self.share_links().join("\n"))
            }
        }
    }

    /// mihomo（Clash Meta）配置：每个入口一个 VLESS 节点，外加手动选择与自动测速两个分组。
    pub fn clash_config(&self) -> String {
        let node = &self.template;
        let fingerprint = param(node, "fp");
        let insecure = matches!(
            param(node, "allowInsecure").as_deref(),
            Some("1" | "true" | "True")
        );
        let names: Vec<String> = self.endpoints.iter().map(Endpoint::label).collect();

        let mut out = String::new();
        let _ = writeln!(
            out,
            "# {} · {} · {} 个节点",
            one_line(&self.title),
            one_line(&self.updated),
            names.len()
        );
        let _ = writeln!(
            out,
            "# CF Best 生成：订阅节点「{}」× 隧道验证通过的优选 IP",
            one_line(&node.name)
        );
        out.push_str(
            "\nmixed-port: 7890\nallow-lan: false\nmode: rule\nlog-level: info\n\nproxies:\n",
        );
        for (endpoint, name) in self.endpoints.iter().zip(&names) {
            let _ = writeln!(out, "  - name: {}", yaml(name));
            out.push_str("    type: vless\n");
            let _ = writeln!(out, "    server: {}", yaml(&endpoint.ip));
            let _ = writeln!(out, "    port: {}", endpoint.port);
            let _ = writeln!(out, "    uuid: {}", yaml(node.uuid.trim()));
            out.push_str("    udp: true\n    tls: true\n");
            let _ = writeln!(out, "    servername: {}", yaml(&node.sni));
            if let Some(fingerprint) = &fingerprint {
                let _ = writeln!(out, "    client-fingerprint: {}", yaml(fingerprint));
            }
            if insecure {
                out.push_str("    skip-cert-verify: true\n");
            }
            out.push_str("    network: ws\n    ws-opts:\n");
            let _ = writeln!(out, "      path: {}", yaml(&node.path));
            let _ = writeln!(out, "      headers:\n        Host: {}", yaml(&node.host));
        }

        out.push_str("\nproxy-groups:\n");
        let _ = writeln!(
            out,
            "  - name: {}\n    type: select\n    proxies:",
            yaml(GROUP_SELECT)
        );
        let _ = writeln!(out, "      - {}", yaml(GROUP_AUTO));
        for name in &names {
            let _ = writeln!(out, "      - {}", yaml(name));
        }
        let _ = writeln!(
            out,
            "  - name: {}\n    type: url-test\n    url: {}\n    interval: 300\n    tolerance: 50\n    proxies:",
            yaml(GROUP_AUTO),
            yaml(TEST_URL)
        );
        for name in &names {
            let _ = writeln!(out, "      - {}", yaml(name));
        }

        out.push_str("\nrules:\n");
        for rule in DIRECT_RULES {
            let _ = writeln!(out, "  - {}", yaml(rule));
        }
        let _ = writeln!(out, "  - {}", yaml(&format!("MATCH,{GROUP_SELECT}")));
        out
    }
}

/// 套用入口地址后的 vless:// 链接。地址换成 IP 后 SNI 与 Host 必须显式写出（原链接省略时默认取
/// 原地址）；隧道验证走的是 TLS，所以固定 security=tls；fp、alpn、path 中的 ed 等其余参数原样保留。
pub fn share_link(node: &VlessNode, endpoint: &Endpoint) -> String {
    let mut params = node.params.clone();
    if !params
        .iter()
        .any(|(key, value)| key == "encryption" && !value.is_empty())
    {
        set_param(&mut params, "encryption", "none");
    }
    set_param(&mut params, "security", "tls");
    set_param(&mut params, "sni", &node.sni);
    set_param(&mut params, "type", "ws");
    set_param(&mut params, "host", &node.host);
    set_param(&mut params, "path", &node.path);
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let host = if endpoint.ip.contains(':') {
        format!("[{}]", endpoint.ip)
    } else {
        endpoint.ip.clone()
    };
    format!(
        "vless://{}@{host}:{}?{query}#{}",
        percent_encode(node.uuid.trim()),
        endpoint.port,
        percent_encode(&endpoint.label())
    )
}

/// 覆盖第一个同名参数并删掉其余重复项，没有则追加在末尾，其他参数的顺序不变。
fn set_param(params: &mut Vec<(String, String)>, key: &str, value: &str) {
    let mut found = false;
    params.retain_mut(|(name, slot)| {
        if name != key {
            return true;
        }
        if found {
            return false;
        }
        found = true;
        *slot = value.to_string();
        true
    });
    if !found {
        params.push((key.to_string(), value.to_string()));
    }
}

fn param(node: &VlessNode, key: &str) -> Option<String> {
    node.params
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// RFC 3986 百分号编码：只保留非保留字符，链接参数与 URL Scheme 都用它。
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// YAML 双引号字符串。JSON 字符串字面量本身就是合法的 YAML 双引号标量，转义交给 serde_json。
fn yaml(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

/// 写进注释的文字不能带换行，否则后半截会被当成配置。
fn one_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::http::parse_vless_link;

    fn template() -> VlessNode {
        parse_vless_link(
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@edge.example.com:443?encryption=none&security=tls&fp=chrome&type=ws&path=%2Fw-123%3Fed%3D2048#HK%20node",
        )
        .unwrap()
    }

    fn endpoint(ip: &str, port: u16, colo: &str) -> Endpoint {
        Endpoint {
            ip: ip.into(),
            port,
            colo: colo.into(),
        }
    }

    #[test]
    fn share_link_swaps_address_and_spells_out_sni_and_host() {
        let link = share_link(&template(), &endpoint("104.16.1.2", 2053, "HKG"));
        assert_eq!(
            link,
            "vless://b831381d-6324-4d53-ad4f-8cda48b30811@104.16.1.2:2053?encryption=none&security=tls&fp=chrome&type=ws&path=%2Fw-123%3Fed%3D2048&sni=edge.example.com&host=edge.example.com#HKG%20104.16.1.2%3A2053"
        );
        // 生成的链接还能被解析回同一个节点，只是入口换成了优选 IP。
        let parsed = parse_vless_link(&link).unwrap();
        assert_eq!(parsed.addr, "104.16.1.2");
        assert_eq!(parsed.port, 2053);
        assert_eq!(parsed.sni, "edge.example.com");
        assert_eq!(parsed.host, "edge.example.com");
        assert_eq!(parsed.path, "/w-123?ed=2048");
        assert_eq!(parsed.name, "HKG 104.16.1.2:2053");
    }

    #[test]
    fn share_link_forces_tls_and_drops_duplicate_params() {
        let mut node = template();
        node.params = vec![
            ("security".into(), "none".into()),
            ("sni".into(), "old.example.com".into()),
            ("sni".into(), "older.example.com".into()),
        ];
        let link = share_link(&node, &endpoint("104.17.0.9", 443, ""));
        assert!(link.contains(
            "?security=tls&sni=edge.example.com&encryption=none&type=ws&host=edge.example.com&path="
        ));
        assert!(link.ends_with("#104.17.0.9"));
        assert_eq!(link.matches("sni=").count(), 1);
    }

    #[test]
    fn feed_requires_websocket_uuid_and_dedupes_endpoints() {
        let endpoints = || {
            vec![
                endpoint("104.16.1.2", 443, "HKG"),
                endpoint("104.16.1.2", 443, "HKG"),
                endpoint("104.16.1.2", 8443, "HKG"),
            ]
        };
        let feed = ProxyFeed::new("CF Best", "now", template(), endpoints()).unwrap();
        assert_eq!(feed.endpoints.len(), 2);

        let mut tcp = template();
        tcp.transport = "tcp".into();
        assert!(ProxyFeed::new("CF Best", "now", tcp, endpoints()).is_none());
        let mut no_uuid = template();
        no_uuid.uuid = " ".into();
        assert!(ProxyFeed::new("CF Best", "now", no_uuid, endpoints()).is_none());
        assert!(ProxyFeed::new("CF Best", "now", template(), Vec::new()).is_none());
    }

    #[test]
    fn clash_config_lists_every_endpoint_in_both_groups() {
        let feed = ProxyFeed::new(
            "CF Best 自动优选",
            "2026-10-29 09:00",
            template(),
            vec![
                endpoint("104.16.1.2", 443, "HKG"),
                endpoint("172.64.3.4", 2053, "SJC"),
            ],
        )
        .unwrap();
        let yaml = feed.clash_config();
        assert!(yaml.starts_with("# CF Best 自动优选 · 2026-10-29 09:00 · 2 个节点\n"));
        assert!(yaml.contains(
            "  - name: \"SJC 172.64.3.4:2053\"\n    type: vless\n    server: \"172.64.3.4\"\n    port: 2053\n    uuid: \"b831381d-6324-4d53-ad4f-8cda48b30811\"\n    udp: true\n    tls: true\n    servername: \"edge.example.com\"\n    client-fingerprint: \"chrome\"\n    network: ws\n    ws-opts:\n      path: \"/w-123?ed=2048\"\n      headers:\n        Host: \"edge.example.com\"\n"
        ));
        // 每个节点在两个分组里各出现一次，另有一次是节点定义本身。
        assert_eq!(yaml.matches("\"HKG 104.16.1.2\"").count(), 3);
        assert!(yaml.contains("      - \"自动选择\"\n      - \"HKG 104.16.1.2\"\n"));
        assert!(yaml.ends_with("  - \"GEOIP,CN,DIRECT\"\n  - \"MATCH,CF Best\"\n"));
        assert!(!yaml.contains("skip-cert-verify"));
    }

    #[test]
    fn clash_config_keeps_names_with_quotes_and_newlines_on_one_line() {
        let mut node = template();
        node.name = "evil\nproxies: []".into();
        node.host = "a\"b".into();
        let feed = ProxyFeed::new("t", "now", node, vec![endpoint("104.16.1.2", 443, "")]).unwrap();
        let yaml = feed.clash_config();
        assert!(yaml.contains("# CF Best 生成：订阅节点「evil proxies: []」"));
        assert!(yaml.contains("        Host: \"a\\\"b\"\n"));
        assert_eq!(yaml.matches("\nproxies:").count(), 1);
    }

    #[test]
    fn v2ray_feed_is_base64_of_share_links() {
        let feed = ProxyFeed::new(
            "t",
            "now",
            template(),
            vec![
                endpoint("104.16.1.2", 443, "HKG"),
                endpoint("104.16.1.3", 443, "HKG"),
            ],
        )
        .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(feed.render(FeedFormat::V2ray))
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert_eq!(text, feed.share_links().join("\n"));
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn percent_encode_keeps_only_unreserved_characters() {
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(
            percent_encode("http://127.0.0.1:1/x?y=z&w"),
            "http%3A%2F%2F127.0.0.1%3A1%2Fx%3Fy%3Dz%26w"
        );
        assert_eq!(percent_encode("自动"), "%E8%87%AA%E5%8A%A8");
    }
}
