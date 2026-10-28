//! Windows 任务栏按钮上的进度显示（`ITaskbarList3`）。
//!
//! 只在主线程使用；GPUI 已在主线程初始化 OLE/COM。创建失败时所有调用都是空操作。

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaskbarProgress {
    None,
    Indeterminate,
    /// 千分比 0..=1000，量化后避免频繁调用 COM。
    Value(u16),
}

impl TaskbarProgress {
    pub fn from_fraction(fraction: Option<f32>, running: bool) -> Self {
        match (running, fraction) {
            (false, _) => TaskbarProgress::None,
            (true, Some(fraction)) => {
                TaskbarProgress::Value((fraction.clamp(0., 1.) * 1000.0).round() as u16)
            }
            (true, None) => TaskbarProgress::Indeterminate,
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows_sys::core::GUID;

    use super::TaskbarProgress;

    const CLSID_TASKBAR_LIST: GUID = GUID::from_u128(0x56fdf344_fd6d_11d0_958a_006097c9a090);
    const IID_ITASKBAR_LIST3: GUID = GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eefaf);

    const TBPF_NOPROGRESS: u32 = 0x0;
    const TBPF_INDETERMINATE: u32 = 0x1;
    const TBPF_NORMAL: u32 = 0x2;

    /// `ITaskbarList3` 虚表的前 11 项（IUnknown → ITaskbarList → ITaskbarList2 → ITaskbarList3）。
    #[repr(C)]
    struct Vtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        hr_init: unsafe extern "system" fn(*mut c_void) -> i32,
        _add_tab: usize,
        _delete_tab: usize,
        _activate_tab: usize,
        _set_active_alt: usize,
        _mark_fullscreen_window: usize,
        set_progress_value: unsafe extern "system" fn(*mut c_void, HWND, u64, u64) -> i32,
        set_progress_state: unsafe extern "system" fn(*mut c_void, HWND, u32) -> i32,
    }

    pub struct Taskbar {
        ptr: *mut c_void,
        last: TaskbarProgress,
    }

    impl Taskbar {
        pub fn new() -> Option<Self> {
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                CoCreateInstance(
                    &CLSID_TASKBAR_LIST,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_ITASKBAR_LIST3,
                    &mut ptr,
                )
            };
            if hr < 0 || ptr.is_null() {
                return None;
            }
            let taskbar = Taskbar {
                ptr,
                last: TaskbarProgress::None,
            };
            if unsafe { (taskbar.vtbl().hr_init)(ptr) } < 0 {
                return None;
            }
            Some(taskbar)
        }

        fn vtbl(&self) -> &Vtbl {
            unsafe { &**(self.ptr as *const *const Vtbl) }
        }

        pub fn set(&mut self, hwnd: HWND, progress: TaskbarProgress) {
            if progress == self.last {
                return;
            }
            self.last = progress;
            let vtbl = self.vtbl();
            unsafe {
                match progress {
                    TaskbarProgress::None => {
                        (vtbl.set_progress_state)(self.ptr, hwnd, TBPF_NOPROGRESS);
                    }
                    TaskbarProgress::Indeterminate => {
                        (vtbl.set_progress_state)(self.ptr, hwnd, TBPF_INDETERMINATE);
                    }
                    TaskbarProgress::Value(permille) => {
                        (vtbl.set_progress_state)(self.ptr, hwnd, TBPF_NORMAL);
                        (vtbl.set_progress_value)(self.ptr, hwnd, permille as u64, 1000);
                    }
                }
            }
        }
    }

    impl Drop for Taskbar {
        fn drop(&mut self) {
            unsafe { (self.vtbl().release)(self.ptr) };
        }
    }
}

#[cfg(target_os = "windows")]
pub use imp::Taskbar;

/// 根据窗口句柄更新任务栏进度；非 Windows 平台为空操作。
pub fn update(taskbar: &mut Option<Taskbar>, window: &gpui::Window, progress: TaskbarProgress) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Some(taskbar) = taskbar.as_mut() else {
            return;
        };
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            taskbar.set(handle.hwnd.get() as _, progress);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (taskbar, window, progress);
    }
}

#[cfg(not(target_os = "windows"))]
pub struct Taskbar;

#[cfg(not(target_os = "windows"))]
impl Taskbar {
    pub fn new() -> Option<Self> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::TaskbarProgress;

    #[test]
    fn progress_quantizes_and_respects_running() {
        assert_eq!(
            TaskbarProgress::from_fraction(Some(0.5), false),
            TaskbarProgress::None
        );
        assert_eq!(
            TaskbarProgress::from_fraction(None, true),
            TaskbarProgress::Indeterminate
        );
        assert_eq!(
            TaskbarProgress::from_fraction(Some(0.4567), true),
            TaskbarProgress::Value(457)
        );
        assert_eq!(
            TaskbarProgress::from_fraction(Some(2.0), true),
            TaskbarProgress::Value(1000)
        );
    }
}
