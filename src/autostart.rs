//! 开机自启：Windows 写入当前用户的 Run 注册表项，macOS 写入 `~/Library/LaunchAgents`。
//! 启动命令带 `--background`；以该参数启动且托盘可用时，窗口直接隐藏到托盘。

pub const BACKGROUND_ARG: &str = "--background";

/// 本次是否由开机自启拉起。
pub fn launched_in_background() -> bool {
    std::env::args().any(|arg| arg == BACKGROUND_ARG)
}

/// 写入或删除开机自启项。开启时使用当前 exe 路径，程序移动位置后再次写入即可更新。
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    imp::set_value("CFBest", enabled)
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn set_value(name: &str, enabled: bool) -> Result<(), String> {
        let key = wide(RUN_KEY);
        let name = wide(name);
        let status = if enabled {
            let exe = std::env::current_exe().map_err(|err| format!("无法获取程序路径：{err}"))?;
            let command = wide(&format!("\"{}\" {}", exe.display(), super::BACKGROUND_ARG));
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("写入注册表失败（错误码 {status}）"))
        }
    }

    #[cfg(test)]
    mod tests {
        /// 读写真实注册表，默认忽略：`cargo test -- --ignored autostart`。
        #[test]
        #[ignore]
        fn autostart_writes_and_removes_run_value() {
            let name = "CFBestAutostartTest";
            super::set_value(name, true).unwrap();
            super::set_value(name, false).unwrap();
            // 已不存在时删除也算成功。
            super::set_value(name, false).unwrap();
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    fn plist_path(name: &str) -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("找不到用户目录")?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("com.aigcfree.{}.plist", name.to_lowercase())))
    }

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    pub fn set_value(name: &str, enabled: bool) -> Result<(), String> {
        let path = plist_path(name)?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("删除启动项失败：{err}"))
                }
                _ => Ok(()),
            };
        }
        let exe = std::env::current_exe().map_err(|err| format!("无法获取程序路径：{err}"))?;
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.aigcfree.{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>{arg}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
            label = name.to_lowercase(),
            exe = xml_escape(&exe.display().to_string()),
            arg = super::BACKGROUND_ARG,
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| format!("创建启动项目录失败：{err}"))?;
        }
        std::fs::write(&path, plist).map_err(|err| format!("写入启动项失败：{err}"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    pub fn set_value(_: &str, _: bool) -> Result<(), String> {
        Err("当前平台暂不支持开机自启".into())
    }
}
