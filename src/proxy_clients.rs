//! 本机代理软件：检测已安装的客户端，通过它们的 URL Scheme 一键导入本机订阅。
//!
//! Windows 读取注册表中 URL Scheme 的打开命令，按可执行文件名认出客户端；macOS 按「应用程序」
//! 目录里的 .app 判断。链接交给系统打开后，由客户端自己弹出导入确认。

use base64::Engine as _;

use crate::core::proxy::{FeedFormat, percent_encode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkStyle {
    /// `scheme://install-config?url=…&name=…`：Clash 系客户端、Stash、Karing。
    InstallConfig,
    /// `hiddify://import/<url>#<name>`。
    Hiddify,
    /// `shadowrocket://add/sub://<Base64 地址>?remark=<name>`。
    Shadowrocket,
}

struct Known {
    name: &'static str,
    /// 客户端专属的 Scheme。
    schemes: &'static [&'static str],
    /// 同时会注册通用的 `clash://`。
    generic_clash: bool,
    /// Windows 打开命令里的可执行文件名（去掉扩展名、空格与连字符后的小写形式）。
    exe_names: &'static [&'static str],
    /// macOS「应用程序」目录中的 .app 名。
    apps: &'static [&'static str],
    style: LinkStyle,
    format: FeedFormat,
}

/// 支持 VLESS 的客户端，按常用程度排列。Clash for Windows、原版 ClashX 的内核不支持 VLESS，不在其列。
const KNOWN: &[Known] = &[
    Known {
        name: "Clash Verge",
        schemes: &["clash-verge"],
        generic_clash: true,
        exe_names: &["clashverge"],
        apps: &["Clash Verge.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
    Known {
        name: "Clash Party",
        schemes: &["mihomo"],
        generic_clash: true,
        exe_names: &["clashparty", "mihomoparty"],
        apps: &["Clash Party.app", "Mihomo Party.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
    Known {
        name: "FlClash",
        schemes: &["flclash"],
        generic_clash: true,
        exe_names: &["flclash"],
        apps: &["FlClash.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
    Known {
        name: "Clash Nyanpasu",
        schemes: &[],
        generic_clash: true,
        exe_names: &["clashnyanpasu"],
        apps: &["Clash Nyanpasu.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
    Known {
        name: "Karing",
        schemes: &["karing"],
        generic_clash: false,
        exe_names: &["karing"],
        apps: &["Karing.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::V2ray,
    },
    Known {
        name: "Stash",
        schemes: &["stash"],
        generic_clash: true,
        exe_names: &["stash"],
        apps: &["Stash.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
    Known {
        name: "Hiddify",
        schemes: &["hiddify"],
        generic_clash: false,
        exe_names: &["hiddify"],
        apps: &["Hiddify.app"],
        style: LinkStyle::Hiddify,
        format: FeedFormat::V2ray,
    },
    Known {
        name: "Shadowrocket",
        schemes: &["shadowrocket"],
        generic_clash: false,
        exe_names: &[],
        apps: &["Shadowrocket.app"],
        style: LinkStyle::Shadowrocket,
        format: FeedFormat::V2ray,
    },
    Known {
        name: "ClashX Meta",
        schemes: &[],
        generic_clash: true,
        exe_names: &[],
        apps: &["ClashX Meta.app"],
        style: LinkStyle::InstallConfig,
        format: FeedFormat::Clash,
    },
];

/// 内核不支持 VLESS 的 Clash 客户端：即使占用了 `clash://` 也不提供导入。
const UNSUPPORTED_EXE_NAMES: &[&str] = &["clashforwindows", "clashx"];

/// 一个可以一键导入的本机客户端。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyClient {
    pub name: String,
    /// 订阅内容的格式。
    pub format: FeedFormat,
    scheme: &'static str,
    style: LinkStyle,
}

impl ProxyClient {
    /// 让客户端导入 `url` 处订阅的链接，`title` 是导入后的订阅名。
    /// `name` 必须放在 `url` 前面：Clash Verge Rev 把 `url=` 之后直到结尾的内容都当作订阅地址。
    pub fn import_link(&self, url: &str, title: &str) -> String {
        match self.style {
            LinkStyle::InstallConfig => format!(
                "{}://install-config?name={}&url={}",
                self.scheme,
                percent_encode(title),
                percent_encode(url)
            ),
            LinkStyle::Hiddify => format!("hiddify://import/{url}#{}", percent_encode(title)),
            LinkStyle::Shadowrocket => format!(
                "shadowrocket://add/sub://{}?remark={}",
                base64::engine::general_purpose::STANDARD.encode(url),
                percent_encode(title)
            ),
        }
    }
}

/// 检测本机已安装、支持一键导入的客户端。只读注册表 / 文件系统，耗时可忽略。
pub fn detect() -> Vec<ProxyClient> {
    imp::detect()
}

/// 交给系统打开 URL Scheme 链接。
pub fn open_url(url: &str) -> Result<(), String> {
    imp::open_url(url)
}

/// 可执行文件名的比较形式：去掉目录、扩展名、空格、连字符与下划线，转小写。
fn exe_key(exe: &str) -> String {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem,
        _ => file,
    };
    stem.chars()
        .filter(|ch| !matches!(ch, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// 从打开命令中取出可执行文件路径：`"C:\a b\x.exe" "%1"` 或不带引号的 `C:\a b\x.exe "%1"`。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn exe_from_command(command: &str) -> Option<String> {
    let command = command.trim();
    let exe = if let Some(rest) = command.strip_prefix('"') {
        rest.split('"').next()?
    } else if let Some(end) = command.to_ascii_lowercase().find(".exe") {
        &command[..end + 4]
    } else {
        command.split_whitespace().next()?
    };
    (!exe.is_empty()).then(|| exe.to_string())
}

/// 根据各 Scheme 的打开程序认出客户端。`exe_for` 返回 Scheme 对应的、确实存在的可执行文件。
/// 通用的 `clash://` 归属于哪个客户端由其可执行文件名决定；该客户端优先用 `clash://`，
/// 因为它是各家都支持的导入方式，专属 Scheme 只在 `clash://` 被别的程序占用时使用。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn from_handlers(exe_for: impl Fn(&str) -> Option<String>) -> Vec<ProxyClient> {
    let generic =
        exe_for("clash").filter(|exe| !UNSUPPORTED_EXE_NAMES.contains(&exe_key(exe).as_str()));
    let mut seen: Vec<String> = Vec::new();
    let mut clients = Vec::new();
    for known in KNOWN {
        let owns_generic = known.generic_clash
            && generic
                .as_deref()
                .is_some_and(|exe| known.exe_names.contains(&exe_key(exe).as_str()));
        let found = if owns_generic {
            generic.clone().map(|exe| ("clash", exe))
        } else {
            known
                .schemes
                .iter()
                .find_map(|scheme| exe_for(scheme).map(|exe| (*scheme, exe)))
        };
        let Some((scheme, exe)) = found else {
            continue;
        };
        let key = exe.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        clients.push(ProxyClient {
            name: known.name.to_string(),
            format: known.format,
            scheme,
            style: known.style,
        });
    }
    // 认不出的程序占用了 clash://：当作通用 Clash 客户端，名字取可执行文件名。
    if let Some(exe) = generic
        && !seen.contains(&exe.to_lowercase())
    {
        let file = exe.rsplit(['\\', '/']).next().unwrap_or(&exe);
        let name = file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".EXE"))
            .unwrap_or(file);
        clients.push(ProxyClient {
            name: name.to_string(),
            format: FeedFormat::Clash,
            scheme: "clash",
            style: LinkStyle::InstallConfig,
        });
    }
    clients
}

/// macOS：按已安装的 .app 认出客户端，优先用专属 Scheme，确保打开的是这个应用。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn from_apps(installed: impl Fn(&str) -> bool) -> Vec<ProxyClient> {
    KNOWN
        .iter()
        .filter(|known| known.apps.iter().any(|app| installed(app)))
        .map(|known| ProxyClient {
            name: known.name.to_string(),
            format: known.format,
            scheme: known.schemes.first().copied().unwrap_or("clash"),
            style: known.style,
        })
        .collect()
}

#[cfg(target_os = "windows")]
mod imp {
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    use super::ProxyClient;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// URL Scheme 的打开命令（HKCR 已合并当前用户与本机的注册），例如 `"C:\…\clash-verge.exe" "%1"`。
    fn handler_command(scheme: &str) -> Option<String> {
        let key = wide(&format!(r"{scheme}\shell\open\command"));
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                key.as_ptr(),
                null(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS || size < 2 {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        let status = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                key.as_ptr(),
                null(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    pub fn detect() -> Vec<ProxyClient> {
        super::from_handlers(|scheme| {
            handler_command(scheme)
                .and_then(|command| super::exe_from_command(&command))
                // 卸载后残留的注册项指向不存在的文件。
                .filter(|exe| Path::new(exe).exists())
        })
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        let verb = wide("open");
        let file = wide(url);
        let result = unsafe {
            ShellExecuteW(
                null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW 成功时返回大于 32 的值。
        let code = result as isize;
        if code > 32 {
            Ok(())
        } else {
            Err(format!("系统无法打开导入链接（错误码 {code}）"))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    use super::ProxyClient;

    pub fn detect() -> Vec<ProxyClient> {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        super::from_apps(|app| roots.iter().any(|root| root.join(app).exists()))
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        let status = std::process::Command::new("open")
            .arg(url)
            .status()
            .map_err(|err| format!("无法打开导入链接：{err}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("没有应用能打开导入链接，客户端可能未注册该 URL Scheme".into())
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::ProxyClient;

    pub fn detect() -> Vec<ProxyClient> {
        Vec::new()
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("无法打开导入链接：{err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handlers(map: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: Vec<(String, String)> = map
            .iter()
            .map(|(scheme, exe)| (scheme.to_string(), exe.to_string()))
            .collect();
        move |scheme| {
            map.iter()
                .find(|(name, _)| name == scheme)
                .map(|(_, exe)| exe.clone())
        }
    }

    fn names(clients: &[ProxyClient]) -> Vec<(&str, &str)> {
        clients
            .iter()
            .map(|client| (client.name.as_str(), client.scheme))
            .collect()
    }

    #[test]
    fn parses_exe_from_open_commands() {
        assert_eq!(
            exe_from_command(r#""C:\Program Files\Stash\Stash.exe" "%1""#).as_deref(),
            Some(r"C:\Program Files\Stash\Stash.exe")
        );
        assert_eq!(
            exe_from_command(r#"C:\Program Files\Clash Verge\clash-verge.exe "%1""#).as_deref(),
            Some(r"C:\Program Files\Clash Verge\clash-verge.exe")
        );
        assert_eq!(exe_from_command("  ").as_deref(), None);
        assert_eq!(
            exe_key(r"C:\Program Files\Clash Verge\clash-verge.exe"),
            "clashverge"
        );
        assert_eq!(exe_key(r"C:\x\Clash Party.EXE"), "clashparty");
        assert_eq!(exe_key("/Applications/FlClash"), "flclash");
    }

    #[test]
    fn generic_clash_goes_to_its_owner_and_others_use_own_schemes() {
        // 本机实际情况：clash:// 与 clash-verge:// 都归 Clash Verge，Stash 的 clash:// 被覆盖。
        let clients = from_handlers(handlers(&[
            ("clash", r"C:\Program Files\Clash Verge\clash-verge.exe"),
            (
                "clash-verge",
                r"C:\Program Files\Clash Verge\clash-verge.exe",
            ),
            ("karing", r"C:\Program Files\Karing\karing.exe"),
            ("stash", r"C:\Program Files\Stash\Stash.exe"),
        ]));
        assert_eq!(
            names(&clients),
            vec![
                ("Clash Verge", "clash"),
                ("Karing", "karing"),
                ("Stash", "stash")
            ]
        );
        assert_eq!(clients[0].format, FeedFormat::Clash);
        assert_eq!(clients[1].format, FeedFormat::V2ray);
    }

    #[test]
    fn unknown_clash_handler_is_kept_but_unsupported_cores_are_skipped() {
        let clients = from_handlers(handlers(&[("clash", r"D:\Apps\MyClash\my-clash.exe")]));
        assert_eq!(names(&clients), vec![("my-clash", "clash")]);

        let clients = from_handlers(handlers(&[(
            "clash",
            r"C:\Program Files\Clash for Windows\Clash for Windows.exe",
        )]));
        assert!(clients.is_empty());

        // clash:// 被不支持 VLESS 的客户端占用时，FlClash 改用自己的 Scheme。
        let clients = from_handlers(handlers(&[
            (
                "clash",
                r"C:\Program Files\Clash for Windows\Clash for Windows.exe",
            ),
            ("flclash", r"C:\Program Files\FlClash\FlClash.exe"),
        ]));
        assert_eq!(names(&clients), vec![("FlClash", "flclash")]);
    }

    #[test]
    fn macos_prefers_app_specific_schemes() {
        let clients = from_apps(|app| {
            ["Clash Verge.app", "Shadowrocket.app", "ClashX Meta.app"].contains(&app)
        });
        assert_eq!(
            names(&clients),
            vec![
                ("Clash Verge", "clash-verge"),
                ("Shadowrocket", "shadowrocket"),
                ("ClashX Meta", "clash")
            ]
        );
    }

    #[test]
    fn builds_import_links_for_each_style() {
        let url = "http://127.0.0.1:27207/abc/daily/clash";
        let clash = ProxyClient {
            name: "Clash Verge".into(),
            format: FeedFormat::Clash,
            scheme: "clash",
            style: LinkStyle::InstallConfig,
        };
        assert_eq!(
            clash.import_link(url, "CF Best 自动优选"),
            "clash://install-config?name=CF%20Best%20%E8%87%AA%E5%8A%A8%E4%BC%98%E9%80%89&url=http%3A%2F%2F127.0.0.1%3A27207%2Fabc%2Fdaily%2Fclash"
        );
        let hiddify = ProxyClient {
            style: LinkStyle::Hiddify,
            scheme: "hiddify",
            ..clash.clone()
        };
        assert_eq!(
            hiddify.import_link(url, "CF"),
            "hiddify://import/http://127.0.0.1:27207/abc/daily/clash#CF"
        );
        let rocket = ProxyClient {
            style: LinkStyle::Shadowrocket,
            scheme: "shadowrocket",
            ..clash
        };
        assert_eq!(
            rocket.import_link("http://a/b", "CF"),
            "shadowrocket://add/sub://aHR0cDovL2EvYg==?remark=CF"
        );
    }
}
