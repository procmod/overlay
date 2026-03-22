//! visual demo - creates a fake game window and overlays it with ESP, health bars, and text.
//! run with: cargo run --example demo
//! then screenshot the result for the README.

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("this demo only runs on windows");
}

#[cfg(target_os = "windows")]
fn main() -> procmod_overlay::Result<()> {
    use procmod_overlay::{Color, Overlay, OverlayTarget};

    let game = game_window::create("Procmod Demo Game", 800, 600);
    std::thread::sleep(std::time::Duration::from_millis(200));

    let mut overlay = Overlay::new(OverlayTarget::Title("Procmod Demo Game".into()))?;

    let green = Color::rgb(74, 222, 128);
    let red = Color::rgb(248, 113, 113);
    let cyan = Color::rgb(56, 189, 248);
    let white = Color::WHITE;
    let dim = Color::rgb(30, 30, 30);

    for _ in 0..120 {
        overlay.begin_frame()?;

        // enemy ESP boxes
        overlay.esp_box(280.0, 140.0, 60.0, 130.0, red, Some("Enemy [85HP]"));
        overlay.health_bar(280.0, 275.0, 60.0, 6.0, 0.85, red, dim);

        overlay.esp_box(520.0, 180.0, 50.0, 110.0, red, Some("Enemy [42HP]"));
        overlay.health_bar(520.0, 295.0, 50.0, 6.0, 0.42, red, dim);

        // friendly
        overlay.esp_box(140.0, 200.0, 55.0, 120.0, green, Some("Ally [100HP]"));
        overlay.health_bar(140.0, 325.0, 55.0, 6.0, 1.0, green, dim);

        // crosshair
        overlay.crosshair(400.0, 300.0, 24.0, 2.0, cyan);

        // HUD text
        overlay.text(16.0, 16.0, "procmod-overlay demo", 20.0, white);
        overlay.text(16.0, 42.0, "FPS: 60", 14.0, Color::rgb(160, 160, 160));

        overlay.end_frame()?;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    drop(overlay);
    drop(game);
    Ok(())
}

#[cfg(target_os = "windows")]
mod game_window {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, EndPaint, FillRect, PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub struct GameWindow {
        hwnd: HWND,
    }

    impl Drop for GameWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    pub fn create(title: &str, w: i32, h: i32) -> GameWindow {
        let class_name = wide(title);
        let hinstance = unsafe { GetModuleHandleW(None).unwrap() };

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hbrBackground: unsafe { CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x1E1E1E)) },
            ..Default::default()
        };

        unsafe { RegisterClassExW(&wc) };

        let title_wide = wide(title);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100, 100, w, h,
                None, None, Some(hinstance.into()), None,
            )
        }
        .expect("failed to create game window");

        GameWindow { hwnd }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let dark = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x1E1E1E));
                FillRect(hdc, &ps.rcPaint, dark);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }
}
