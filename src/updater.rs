//! 安装更新。Windows：把正在运行的 exe 改名让位，放入新版本后重启——Windows 允许给运行中的
//! exe 改名，但不允许覆盖或删除它；macOS：把 dmg 存到「下载」并打开，由用户拖入「应用程序」
//! （替换运行中的 .app 容易破坏签名，交给 Finder 更稳妥）。

use std::path::PathBuf;

/// 新版本由旧版本拉起时带上这个参数：等旧进程退出、拿到单实例锁后再启动，并清理改名留下的旧文件。
pub const AFTER_UPDATE_ARG: &str = "--after-update";

/// 本次是否由更新拉起。
pub fn launched_after_update() -> bool {
    std::env::args().any(|arg| arg == AFTER_UPDATE_ARG)
}

pub enum Installed {
    /// 新版本已就位并启动，当前进程应立即退出。只有 Windows 原地替换会走到这里。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Restarting,
    /// 安装包已保存（并尽量打开）在这个位置。
    Saved(PathBuf),
}

/// 安装已校验的安装包，`file_name` 是它在发布页上的文件名。
pub fn install(file_name: &str, bytes: &[u8]) -> Result<Installed, String> {
    imp::install(file_name, bytes)
}

/// 删除上次更新改名留下的旧 exe。旧进程可能还没完全退出，删不掉时在后台重试一会儿。
pub fn cleanup_after_update() {
    imp::cleanup();
}

/// 把安装包存到「下载」文件夹；找不到时退回系统临时目录。
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
fn save_to_downloads(file_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let home = std::env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    });
    let dir = home
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(file_name);
    std::fs::write(&path, bytes).map_err(|err| format!("保存安装包失败：{err}"))?;
    Ok(path)
}

#[cfg(target_os = "windows")]
mod imp {
    use std::time::Duration;

    use super::Installed;
    use crate::core::storage;

    pub fn install(file_name: &str, bytes: &[u8]) -> Result<Installed, String> {
        let exe = std::env::current_exe().map_err(|err| format!("无法获取程序路径：{err}"))?;
        let staged = exe.with_extension("exe.new");
        let old = exe.with_extension("exe.old");
        // 程序所在目录不可写（例如放在 Program Files 下）：保存到「下载」文件夹并打开它。
        if std::fs::write(&staged, bytes).is_err() {
            let path = super::save_to_downloads(file_name, bytes)?;
            if let Some(dir) = path.parent() {
                storage::open_folder(dir);
            }
            return Ok(Installed::Saved(path));
        }
        let _ = std::fs::remove_file(&old);
        if let Err(err) = std::fs::rename(&exe, &old) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("无法替换正在运行的程序：{err}"));
        }
        if let Err(err) = std::fs::rename(&staged, &exe) {
            let _ = std::fs::rename(&old, &exe);
            return Err(format!("放入新版本失败：{err}"));
        }
        std::process::Command::new(&exe)
            .arg(super::AFTER_UPDATE_ARG)
            .spawn()
            .map_err(|err| format!("新版本已就位，但启动失败：{err}；请手动打开 CF Best"))?;
        Ok(Installed::Restarting)
    }

    pub fn cleanup() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let old = exe.with_extension("exe.old");
        if !old.exists() {
            return;
        }
        std::thread::spawn(move || {
            for _ in 0..30 {
                if std::fs::remove_file(&old).is_ok() || !old.exists() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            storage::log_line(&format!("删除旧版本文件失败：{}", old.display()));
        });
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Installed;

    pub fn install(file_name: &str, bytes: &[u8]) -> Result<Installed, String> {
        let path = super::save_to_downloads(file_name, bytes)?;
        let opened = std::process::Command::new("open")
            .arg(&path)
            .status()
            .is_ok_and(|status| status.success());
        if !opened {
            return Err(format!("安装包已保存到 {}，但无法自动打开", path.display()));
        }
        Ok(Installed::Saved(path))
    }

    pub fn cleanup() {}
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::Installed;

    pub fn install(_: &str, _: &[u8]) -> Result<Installed, String> {
        Err("当前平台暂不支持自动更新，请到发布页下载".into())
    }

    pub fn cleanup() {}
}
