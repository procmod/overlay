use crate::error::{Error, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

#[repr(C)]
struct MARGINS {
    cxLeftWidth: i32,
    cxRightWidth: i32,
    cyTopHeight: i32,
    cyBottomHeight: i32,
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmExtendFrameIntoClientArea(hwnd: HWND, pmarinset: *const MARGINS) -> i32;
}

/// How to find the target game window.
pub enum OverlayTarget {
    /// Find by window title substring.
    Title(String),
    /// Find by window class name.
    Class(String),
    /// Use a raw HWND directly.
    Hwnd(isize),
    /// Find the primary window of a process by PID.
    Pid(u32),
}

pub(crate) struct OverlayWindow {
    pub hwnd: HWND,
    target: HWND,
    class_atom: u16,
}

impl OverlayWindow {
    pub fn create(target: &OverlayTarget) -> Result<Self> {
        let target_hwnd = find_target(target)?;
        let class_name = wide_string("ProcmodOverlay");
        let hinstance = unsafe { GetModuleHandleW(None).unwrap() };

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        let atom = unsafe { RegisterClassExW(&wc) };
        if atom == 0 {
            return Err(Error::WindowCreation(std::io::Error::last_os_error()));
        }

        let mut target_rect = RECT::default();
        unsafe { GetWindowRect(target_hwnd, &mut target_rect) }
            .map_err(|_| Error::WindowNotFound)?;

        let w = target_rect.right - target_rect.left;
        let h = target_rect.bottom - target_rect.top;

        let ex_style = WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE;
        let title = wide_string("procmod-overlay");
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                target_rect.left,
                target_rect.top,
                w,
                h,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|_| Error::WindowCreation(std::io::Error::last_os_error()))?;

        unsafe {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }

        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        unsafe {
            DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = UpdateWindow(hwnd);
        }

        Ok(Self {
            hwnd,
            target: target_hwnd,
            class_atom: atom,
        })
    }

    /// Update overlay position to match the target window. Returns false if the target is gone.
    pub fn sync_position(&mut self) -> bool {
        if !unsafe { IsWindow(Some(self.target)) }.as_bool() {
            return false;
        }

        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.target, &mut rect).is_err() } {
            return false;
        }

        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                rect.left,
                rect.top,
                w,
                h,
                SWP_NOACTIVATE,
            );
        }
        true
    }

    /// Returns the current width and height of the overlay.
    pub fn size(&self) -> (u32, u32) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        (
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        )
    }

    /// Process pending window messages. Returns false if WM_QUIT was received.
    pub fn pump_messages(&self) -> bool {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, Some(self.hwnd), 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }

    pub fn is_target_visible(&self) -> bool {
        if !unsafe { IsWindow(Some(self.target)) }.as_bool() {
            return false;
        }
        let fg = unsafe { GetForegroundWindow() };
        fg == self.target
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let hinstance = GetModuleHandleW(None).unwrap();
            let _ = UnregisterClassW(
                PCWSTR(self.class_atom as *const u16),
                Some(hinstance.into()),
            );
        }
    }
}

fn find_target(target: &OverlayTarget) -> Result<HWND> {
    match target {
        OverlayTarget::Title(title) => find_window_by_title(title),
        OverlayTarget::Class(class) => {
            let class_wide = wide_string(class);
            let hwnd = unsafe { FindWindowW(PCWSTR(class_wide.as_ptr()), PCWSTR::null()) }
                .map_err(|_| Error::WindowNotFound)?;
            Ok(hwnd)
        }
        OverlayTarget::Hwnd(raw) => {
            let hwnd = HWND(*raw as *mut _);
            if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                return Err(Error::WindowNotFound);
            }
            Ok(hwnd)
        }
        OverlayTarget::Pid(pid) => find_window_by_pid(*pid),
    }
}

fn find_window_by_title(title: &str) -> Result<HWND> {
    struct SearchState {
        query: String,
        result: HWND,
    }

    let mut state = SearchState {
        query: title.to_lowercase(),
        result: HWND::default(),
    };

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf) as usize;
        if len > 0 {
            let text = String::from_utf16_lossy(&buf[..len]).to_lowercase();
            if text.contains(&state.query) {
                state.result = hwnd;
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(
            Some(callback),
            LPARAM(&mut state as *mut SearchState as isize),
        );
    }

    if state.result == HWND::default() {
        Err(Error::WindowNotFound)
    } else {
        Ok(state.result)
    }
}

fn find_window_by_pid(pid: u32) -> Result<HWND> {
    struct SearchState {
        pid: u32,
        result: HWND,
    }

    let mut state = SearchState {
        pid,
        result: HWND::default(),
    };

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == state.pid && IsWindowVisible(hwnd).as_bool() {
            state.result = hwnd;
            return BOOL(0);
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(
            Some(callback),
            LPARAM(&mut state as *mut SearchState as isize),
        );
    }

    if state.result == HWND::default() {
        Err(Error::WindowNotFound)
    } else {
        Ok(state.result)
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}
