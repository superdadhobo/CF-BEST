//! Windows 系统托盘：图标、提示文字、左键恢复窗口、右键菜单与气泡通知。
//!
//! 只在主线程使用；GPUI 的 `GetMessageW` 循环会把消息派发给这里的隐藏窗口。
//! 开启后「最小化 / 关闭」会把主窗口隐藏到托盘，从托盘菜单「退出」才真正结束程序。

use std::sync::atomic::{AtomicBool, Ordering};

/// 当前平台是否有托盘实现（目前只有 Windows）。
pub const SUPPORTED: bool = cfg!(target_os = "windows");

static TRAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 托盘是否在工作（决定最小化 / 关闭是否改为隐藏）。
pub fn active() -> bool {
    TRAY_ACTIVE.load(Ordering::Relaxed)
}

/// 用户是否已从托盘菜单选择退出。
pub fn exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::Relaxed)
}

#[cfg(target_os = "windows")]
mod imp {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        DestroyWindow, GetCursorPos, GetSystemMetrics, HICON, HWND_MESSAGE, IMAGE_ICON, IsIconic,
        IsWindowVisible, LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, PostMessageW,
        RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SW_HIDE, SW_RESTORE,
        SW_SHOW, SetForegroundWindow, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
        WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
        WNDCLASSW,
    };

    use super::{EXIT_REQUESTED, TRAY_ACTIVE};

    const WM_TRAY: u32 = WM_APP + 41;
    const MENU_SHOW: usize = 1;
    const MENU_EXIT: usize = 2;

    static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    thread_local! {
        /// 资源管理器重启（TaskbarCreated）时用来重新添加图标。
        static ICON_DATA: RefCell<Option<NOTIFYICONDATAW>> = const { RefCell::new(None) };
    }

    /// Win32 `MAKEINTRESOURCEW`：以整数 ID 代替资源名指针。
    fn make_int_resource(id: u16) -> *const u16 {
        std::ptr::without_provenance(id as usize)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn copy_into(dst: &mut [u16], text: &str) {
        let units: Vec<u16> = text.encode_utf16().take(dst.len() - 1).collect();
        dst[..units.len()].copy_from_slice(&units);
        dst[units.len()] = 0;
    }

    fn main_hwnd() -> HWND {
        MAIN_HWND.load(Ordering::Relaxed) as HWND
    }

    pub(super) fn show_main_window() {
        let hwnd = main_hwnd();
        if hwnd.is_null() {
            return;
        }
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }

    pub fn hide_main_window() {
        let hwnd = main_hwnd();
        if !hwnd.is_null() {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
    }

    pub fn main_window_hidden() -> bool {
        let hwnd = main_hwnd();
        !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0
    }

    fn request_exit() {
        EXIT_REQUESTED.store(true, Ordering::Relaxed);
        let hwnd = main_hwnd();
        if !hwnd.is_null() {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
    }

    unsafe fn show_menu(hwnd: HWND) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING, MENU_SHOW, wide("显示主窗口").as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, MENU_EXIT, wide("退出 CF Best").as_ptr());
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            // 菜单要求宿主窗口在前台，否则点击外部不会关闭菜单。
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            match command as usize {
                MENU_SHOW => show_main_window(),
                MENU_EXIT => request_exit(),
                _ => {}
            }
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_TRAY {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_main_window(),
                WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_menu(hwnd) },
                _ => {}
            }
            return 0;
        }
        let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
        if taskbar_created != 0 && msg == taskbar_created {
            ICON_DATA.with(|data| {
                if let Some(data) = data.borrow().as_ref() {
                    unsafe { Shell_NotifyIconW(NIM_ADD, data) };
                }
            });
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub struct Tray {
        hwnd: HWND,
        icon: HICON,
        tip: String,
    }

    impl Tray {
        pub fn new(main: HWND, tip: &str) -> Option<Self> {
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class_name = wide("CfBestTrayWindow");
                let class = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance,
                    lpszClassName: class_name.as_ptr(),
                    ..std::mem::zeroed()
                };
                // 重复注册会失败但不影响使用已注册的类。
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    wide("CF Best Tray").as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    return None;
                }
                TASKBAR_CREATED.store(
                    RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                    Ordering::Relaxed,
                );
                // app.rc 中的图标资源 ID 为 1。
                let icon = LoadImageW(
                    instance,
                    make_int_resource(1),
                    IMAGE_ICON,
                    GetSystemMetrics(SM_CXSMICON),
                    GetSystemMetrics(SM_CYSMICON),
                    LR_DEFAULTCOLOR,
                ) as HICON;
                let mut data: NOTIFYICONDATAW = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = hwnd;
                data.uID = 1;
                data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                data.uCallbackMessage = WM_TRAY;
                data.hIcon = icon;
                copy_into(&mut data.szTip, tip);
                if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                    DestroyWindow(hwnd);
                    if !icon.is_null() {
                        DestroyIcon(icon);
                    }
                    return None;
                }
                ICON_DATA.with(|slot| *slot.borrow_mut() = Some(data));
                MAIN_HWND.store(main as isize, Ordering::Relaxed);
                TRAY_HWND.store(hwnd as isize, Ordering::Relaxed);
                TRAY_ACTIVE.store(true, Ordering::Relaxed);
                Some(Self {
                    hwnd,
                    icon,
                    tip: tip.to_string(),
                })
            }
        }

        fn with_data(&self, update: impl FnOnce(&mut NOTIFYICONDATAW)) {
            ICON_DATA.with(|slot| {
                if let Some(data) = slot.borrow_mut().as_mut() {
                    update(data);
                    unsafe { Shell_NotifyIconW(NIM_MODIFY, data) };
                    data.uFlags &= !NIF_INFO;
                }
            });
        }

        pub fn set_tip(&mut self, tip: &str) {
            if self.tip == tip {
                return;
            }
            self.tip = tip.to_string();
            self.with_data(|data| {
                data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                copy_into(&mut data.szTip, tip);
            });
        }

        /// 气泡通知（遵守系统的勿扰时段）。
        pub fn notify(&self, title: &str, text: &str) {
            self.with_data(|data| {
                data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_INFO;
                data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
                copy_into(&mut data.szInfoTitle, title);
                copy_into(&mut data.szInfo, text);
            });
        }
    }

    /// 自检用：向托盘窗口投递一次「左键单击」回调，走真实的消息派发路径。
    #[cfg(debug_assertions)]
    pub fn post_left_click() {
        let hwnd = TRAY_HWND.load(Ordering::Relaxed) as HWND;
        if !hwnd.is_null() {
            unsafe { PostMessageW(hwnd, WM_TRAY, 0, WM_LBUTTONUP as LPARAM) };
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            TRAY_HWND.store(0, Ordering::Relaxed);
            ICON_DATA.with(|slot| {
                if let Some(data) = slot.borrow_mut().take() {
                    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                }
            });
            unsafe {
                DestroyWindow(self.hwnd);
                if !self.icon.is_null() {
                    DestroyIcon(self.icon);
                }
            }
            TRAY_ACTIVE.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(all(target_os = "windows", debug_assertions))]
pub use imp::post_left_click;
#[cfg(target_os = "windows")]
pub use imp::{Tray, hide_main_window, main_window_hidden};

/// 为当前窗口创建托盘图标；非 Windows 平台返回 `None`。
pub fn create(window: &gpui::Window, tip: &str) -> Option<Tray> {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return None;
        };
        Tray::new(handle.hwnd.get() as _, tip)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, tip);
        None
    }
}

#[cfg(not(target_os = "windows"))]
pub struct Tray;

#[cfg(not(target_os = "windows"))]
impl Tray {
    pub fn set_tip(&mut self, _: &str) {}
    pub fn notify(&self, _: &str, _: &str) {}
}

#[cfg(not(target_os = "windows"))]
pub fn hide_main_window() {}

#[cfg(not(target_os = "windows"))]
pub fn main_window_hidden() -> bool {
    false
}
