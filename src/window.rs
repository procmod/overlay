use crate::error::{Error, Result};
use crate::input::{InputEvent, InteractionMode, KeyState, MouseButton};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ScreenToClient, UpdateWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

#[repr(C)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
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

struct WindowState {
    events: VecDeque<InputEvent>,
}

pub(crate) struct OverlayWindow {
    pub hwnd: HWND,
    target: HWND,
    class_atom: u16,
    mode: InteractionMode,
    state: Box<WindowState>,
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
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
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
        let mut state = Box::new(WindowState {
            events: VecDeque::new(),
        });
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
                Some((&mut *state as *mut WindowState).cast()),
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
            mode: InteractionMode::PassThrough,
            state,
        })
    }

    pub fn interaction_mode(&self) -> InteractionMode {
        self.mode
    }

    pub fn set_interaction_mode(&mut self, mode: InteractionMode) -> Result<()> {
        if mode == self.mode {
            return Ok(());
        }
        unsafe {
            let current = WINDOW_EX_STYLE(GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32);
            let next = match mode {
                InteractionMode::PassThrough => current | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                InteractionMode::Interactive => {
                    WINDOW_EX_STYLE(current.0 & !(WS_EX_TRANSPARENT | WS_EX_NOACTIVATE).0)
                }
            };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next.0 as isize);
            let flags = SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_FRAMECHANGED
                | if mode == InteractionMode::PassThrough {
                    SWP_NOACTIVATE
                } else {
                    SET_WINDOW_POS_FLAGS(0)
                };
            SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags)
                .map_err(|_| Error::WindowCreation(std::io::Error::last_os_error()))?;
            let foreground = GetForegroundWindow();
            let foreground_thread = GetWindowThreadProcessId(foreground, None);
            let current_thread = GetCurrentThreadId();
            let attached = foreground_thread != 0
                && foreground_thread != current_thread
                && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
            let focus_target = if mode == InteractionMode::Interactive {
                self.hwnd
            } else {
                let _ = ReleaseCapture();
                self.target
            };
            let activated = SetForegroundWindow(focus_target).as_bool();
            let _ = SetFocus(Some(focus_target));
            if attached {
                let _ = AttachThreadInput(current_thread, foreground_thread, false);
            }
            if !activated {
                return Err(if mode == InteractionMode::Interactive {
                    Error::WindowCreation(std::io::Error::last_os_error())
                } else {
                    Error::TargetWindowLost
                });
            }
        }
        self.state.events.clear();
        self.mode = mode;
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<InputEvent> {
        self.state.events.drain(..).collect()
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
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|_| Error::ProcessNotFound { pid })?;
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(process);
    }

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
        Err(Error::ProcessWindowNotFound { pid })
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
    if msg == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *state_ptr;
    let point = || {
        let x = lparam.0 as i16 as f32;
        let y = (lparam.0 >> 16) as i16 as f32;
        (x, y)
    };
    let button_event = |button, pressed| {
        let (x, y) = point();
        InputEvent::MouseButton {
            button,
            pressed,
            x,
            y,
        }
    };
    let event = match msg {
        WM_MOUSEMOVE => {
            let (x, y) = point();
            Some(InputEvent::MouseMoved { x, y })
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            SetCapture(hwnd);
            Some(button_event(
                match msg {
                    WM_LBUTTONDOWN => MouseButton::Left,
                    WM_RBUTTONDOWN => MouseButton::Right,
                    WM_MBUTTONDOWN => MouseButton::Middle,
                    _ if ((wparam.0 >> 16) & 0xffff) == 1 => MouseButton::X1,
                    _ => MouseButton::X2,
                },
                true,
            ))
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let _ = ReleaseCapture();
            Some(button_event(
                match msg {
                    WM_LBUTTONUP => MouseButton::Left,
                    WM_RBUTTONUP => MouseButton::Right,
                    WM_MBUTTONUP => MouseButton::Middle,
                    _ if ((wparam.0 >> 16) & 0xffff) == 1 => MouseButton::X1,
                    _ => MouseButton::X2,
                },
                false,
            ))
        }
        WM_MOUSEWHEEL => {
            let mut screen = POINT {
                x: lparam.0 as i16 as i32,
                y: (lparam.0 >> 16) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut screen);
            Some(InputEvent::MouseWheel {
                delta: ((wparam.0 >> 16) as i16 as f32) / WHEEL_DELTA as f32,
                x: screen.x as f32,
                y: screen.y as f32,
            })
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(InputEvent::Key {
            virtual_key: wparam.0 as u16,
            state: KeyState::Pressed,
        }),
        WM_KEYUP | WM_SYSKEYUP => Some(InputEvent::Key {
            virtual_key: wparam.0 as u16,
            state: KeyState::Released,
        }),
        WM_CHAR => char::from_u32(wparam.0 as u32).map(InputEvent::Text),
        WM_SETFOCUS => Some(InputEvent::Focused(true)),
        WM_KILLFOCUS => Some(InputEvent::Focused(false)),
        WM_CLOSE => Some(InputEvent::CloseRequested),
        _ => None,
    };
    if let Some(event) = event {
        state.events.push_back(event);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}
