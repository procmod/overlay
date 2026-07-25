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

const MAX_PENDING_EVENTS: usize = 4096;

struct WindowState {
    events: VecDeque<InputEvent>,
    dropped_events: usize,
    pressed_mouse_buttons: u8,
    pending_high_surrogate: Option<u16>,
    last_mouse_position: (f32, f32),
    closed: bool,
}

impl WindowState {
    fn push_event(&mut self, event: InputEvent) {
        if matches!(event, InputEvent::MouseMoved { .. })
            && matches!(self.events.back(), Some(InputEvent::MouseMoved { .. }))
        {
            self.events.pop_back();
        }
        if self.events.len() == MAX_PENDING_EVENTS {
            self.events.pop_front();
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        self.events.push_back(event);
    }

    fn clear_input_state(&mut self) {
        self.pressed_mouse_buttons = 0;
        self.pending_high_surrogate = None;
    }
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
            dropped_events: 0,
            pressed_mouse_buttons: 0,
            pending_high_surrogate: None,
            last_mouse_position: (0.0, 0.0),
            closed: false,
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
            if SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next.0 as isize) == 0 {
                return Err(Error::WindowCreation(std::io::Error::last_os_error()));
            }
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED | SWP_NOACTIVATE;
            if let Err(error) = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags) {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, current.0 as isize);
                let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, flags | SWP_NOZORDER);
                return Err(Error::WindowCreation(std::io::Error::other(error)));
            }

            if mode == InteractionMode::PassThrough {
                let _ = ReleaseCapture();
            }
            self.state.events.clear();
            self.state.clear_input_state();
            self.mode = mode;

            let focus_target = if mode == InteractionMode::Interactive {
                self.hwnd
            } else {
                self.target
            };
            try_activate_window(focus_target);
        }
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<InputEvent> {
        self.state.events.drain(..).collect()
    }

    pub fn take_dropped_event_count(&mut self) -> usize {
        std::mem::take(&mut self.state.dropped_events)
    }

    pub fn close(&mut self) {
        if !self.state.closed {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
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

    /// Process pending window messages. Returns false after the overlay is destroyed.
    pub fn pump_messages(&self) -> bool {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, Some(self.hwnd), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        !self.state.closed
    }

    pub fn is_target_foreground(&self) -> bool {
        unsafe { IsWindow(Some(self.target)).as_bool() && GetForegroundWindow() == self.target }
    }

    pub fn is_overlay_foreground(&self) -> bool {
        unsafe { !self.state.closed && GetForegroundWindow() == self.hwnd }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.state.closed {
                let _ = DestroyWindow(self.hwnd);
            }
            let hinstance = GetModuleHandleW(None).unwrap();
            let _ = UnregisterClassW(
                PCWSTR(self.class_atom as *const u16),
                Some(hinstance.into()),
            );
        }
    }
}

unsafe fn try_activate_window(hwnd: HWND) {
    let foreground = GetForegroundWindow();
    let foreground_thread = GetWindowThreadProcessId(foreground, None);
    let current_thread = GetCurrentThreadId();
    let attached = foreground_thread != 0
        && foreground_thread != current_thread
        && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
    let _ = SetForegroundWindow(hwnd);
    let _ = SetFocus(Some(hwnd));
    if attached {
        let _ = AttachThreadInput(current_thread, foreground_thread, false);
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
    match msg {
        WM_MOUSEMOVE => {
            let position = point();
            state.last_mouse_position = position;
            state.push_event(InputEvent::MouseMoved {
                x: position.0,
                y: position.1,
            });
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            let button = mouse_button(msg, wparam);
            let position = point();
            state.last_mouse_position = position;
            state.pressed_mouse_buttons |= mouse_button_mask(button);
            SetCapture(hwnd);
            state.push_event(mouse_button_event(button, true, position));
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let button = mouse_button(msg, wparam);
            let position = point();
            state.last_mouse_position = position;
            state.pressed_mouse_buttons &= !mouse_button_mask(button);
            if state.pressed_mouse_buttons == 0 {
                let _ = ReleaseCapture();
            }
            state.push_event(mouse_button_event(button, false, position));
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => release_pressed_mouse_buttons(state),
        WM_MOUSEWHEEL => {
            let mut screen = POINT {
                x: lparam.0 as i16 as i32,
                y: (lparam.0 >> 16) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut screen);
            state.push_event(InputEvent::MouseWheel {
                delta: ((wparam.0 >> 16) as i16 as f32) / WHEEL_DELTA as f32,
                x: screen.x as f32,
                y: screen.y as f32,
            });
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => state.push_event(InputEvent::Key {
            virtual_key: wparam.0 as u16,
            state: KeyState::Pressed,
        }),
        WM_KEYUP | WM_SYSKEYUP => state.push_event(InputEvent::Key {
            virtual_key: wparam.0 as u16,
            state: KeyState::Released,
        }),
        WM_CHAR => push_utf16_code_unit(state, wparam.0 as u16),
        WM_SETFOCUS => state.push_event(InputEvent::Focused(true)),
        WM_KILLFOCUS => {
            release_pressed_mouse_buttons(state);
            state.push_event(InputEvent::Focused(false));
        }
        WM_CLOSE => state.push_event(InputEvent::CloseRequested),
        WM_DESTROY => state.closed = true,
        WM_NCDESTROY => {
            state.closed = true;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}

fn mouse_button(message: u32, wparam: WPARAM) -> MouseButton {
    match message {
        WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
        WM_RBUTTONDOWN | WM_RBUTTONUP => MouseButton::Right,
        WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
        _ if ((wparam.0 >> 16) & 0xffff) == 1 => MouseButton::X1,
        _ => MouseButton::X2,
    }
}

fn mouse_button_mask(button: MouseButton) -> u8 {
    1 << match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
    }
}

fn mouse_button_event(button: MouseButton, pressed: bool, position: (f32, f32)) -> InputEvent {
    InputEvent::MouseButton {
        button,
        pressed,
        x: position.0,
        y: position.1,
    }
}

fn release_pressed_mouse_buttons(state: &mut WindowState) {
    unsafe {
        let _ = ReleaseCapture();
    }
    let pressed = std::mem::take(&mut state.pressed_mouse_buttons);
    for button in [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::X1,
        MouseButton::X2,
    ] {
        if pressed & mouse_button_mask(button) != 0 {
            state.push_event(mouse_button_event(button, false, state.last_mouse_position));
        }
    }
}

fn push_utf16_code_unit(state: &mut WindowState, code_unit: u16) {
    if (0xd800..=0xdbff).contains(&code_unit) {
        state.pending_high_surrogate = Some(code_unit);
        return;
    }
    let decoded = if let Some(high) = state.pending_high_surrogate.take() {
        if (0xdc00..=0xdfff).contains(&code_unit) {
            char::decode_utf16([high, code_unit])
                .next()
                .and_then(|result| result.ok())
        } else {
            char::from_u32(code_unit as u32)
        }
    } else {
        char::from_u32(code_unit as u32)
    };
    if let Some(character) = decoded {
        state.push_event(InputEvent::Text(character));
    }
}

fn wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WindowState {
        WindowState {
            events: VecDeque::new(),
            dropped_events: 0,
            pressed_mouse_buttons: 0,
            pending_high_surrogate: None,
            last_mouse_position: (0.0, 0.0),
            closed: false,
        }
    }

    #[test]
    fn consecutive_mouse_moves_are_coalesced() {
        let mut state = state();
        state.push_event(InputEvent::MouseMoved { x: 1.0, y: 2.0 });
        state.push_event(InputEvent::MouseMoved { x: 3.0, y: 4.0 });

        assert_eq!(
            state.events.into_iter().collect::<Vec<_>>(),
            vec![InputEvent::MouseMoved { x: 3.0, y: 4.0 }]
        );
    }

    #[test]
    fn event_queue_is_bounded_and_counts_drops() {
        let mut state = state();
        for virtual_key in 0..=MAX_PENDING_EVENTS {
            state.push_event(InputEvent::Key {
                virtual_key: virtual_key as u16,
                state: KeyState::Pressed,
            });
        }

        assert_eq!(state.events.len(), MAX_PENDING_EVENTS);
        assert_eq!(state.dropped_events, 1);
    }

    #[test]
    fn utf16_surrogate_pair_produces_one_character() {
        let mut state = state();
        push_utf16_code_unit(&mut state, 0xd83d);
        push_utf16_code_unit(&mut state, 0xde80);

        assert_eq!(state.events.pop_front(), Some(InputEvent::Text('🚀')));
        assert!(state.events.is_empty());
    }

    #[test]
    fn releasing_pressed_buttons_preserves_other_events() {
        let mut state = state();
        state.last_mouse_position = (12.0, 34.0);
        state.pressed_mouse_buttons =
            mouse_button_mask(MouseButton::Left) | mouse_button_mask(MouseButton::Right);

        release_pressed_mouse_buttons(&mut state);

        assert_eq!(state.pressed_mouse_buttons, 0);
        assert_eq!(state.events.len(), 2);
        assert!(state.events.iter().all(|event| matches!(
            event,
            InputEvent::MouseButton {
                pressed: false,
                x: 12.0,
                y: 34.0,
                ..
            }
        )));
    }
}
