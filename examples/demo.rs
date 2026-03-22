//! visual demo - creates a dark window and overlays it with ESP boxes, health bars, and text.
//!
//! cargo run --example demo              # interactive, renders for 2 seconds
//! PROCMOD_SCREENSHOT=1 cargo run --example demo  # renders, saves example.png, exits

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("this demo only runs on windows");
}

#[cfg(target_os = "windows")]
fn main() -> procmod_overlay::Result<()> {
    use procmod_overlay::{Color, Overlay, OverlayTarget};

    let screenshot_mode = std::env::var("PROCMOD_SCREENSHOT").is_ok();

    let game = game_window::create("Procmod Demo Game", 800, 600);
    std::thread::sleep(std::time::Duration::from_millis(500));
    game.set_foreground();

    let mut overlay = Overlay::new(OverlayTarget::Title("Procmod Demo Game".into()))?;

    let green = Color::rgb(74, 222, 128);
    let red = Color::rgb(248, 113, 113);
    let cyan = Color::rgb(56, 189, 248);
    let white = Color::WHITE;
    let dim = Color::rgb(30, 30, 30);

    let frames = if screenshot_mode { 30 } else { 120 };

    for _ in 0..frames {
        overlay.begin_frame()?;

        overlay.esp_box(280.0, 140.0, 60.0, 130.0, red, Some("Enemy [85HP]"));
        overlay.health_bar(280.0, 275.0, 60.0, 6.0, 0.85, red, dim);

        overlay.esp_box(520.0, 180.0, 50.0, 110.0, red, Some("Enemy [42HP]"));
        overlay.health_bar(520.0, 295.0, 50.0, 6.0, 0.42, red, dim);

        overlay.esp_box(140.0, 200.0, 55.0, 120.0, green, Some("Ally [100HP]"));
        overlay.health_bar(140.0, 325.0, 55.0, 6.0, 1.0, green, dim);

        overlay.crosshair(400.0, 300.0, 24.0, 2.0, cyan);

        overlay.text(16.0, 16.0, "procmod-overlay demo", 20.0, white);
        overlay.text(16.0, 42.0, "FPS: 60", 14.0, Color::rgb(160, 160, 160));

        overlay.end_frame()?;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    if screenshot_mode {
        let (x, y) = game.position();
        let (w, h) = game.size();
        screenshot::capture_and_save("example.png", x, y, w, h);
        eprintln!("saved example.png ({}x{})", w, h);
    }

    drop(overlay);
    drop(game);
    Ok(())
}

#[cfg(target_os = "windows")]
mod screenshot {
    use windows::Win32::Graphics::Gdi::*;

    pub fn capture_and_save(path: &str, x: i32, y: i32, w: i32, h: i32) {
        let pixels = capture(x, y, w, h);

        let file = std::fs::File::create(path).expect("failed to create screenshot file");
        let buf = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(buf, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("failed to write png header");
        writer
            .write_image_data(&pixels)
            .expect("failed to write png data");
    }

    fn capture(x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        unsafe {
            let hdc_screen = GetDC(None);
            let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
            let hbm = CreateCompatibleBitmap(hdc_screen, w, h);
            let old = SelectObject(hdc_mem, hbm.into());

            let _ = BitBlt(hdc_mem, 0, 0, w, h, Some(hdc_screen), x, y, SRCCOPY);

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0 as u32,
                    ..std::mem::zeroed()
                },
                ..std::mem::zeroed()
            };

            let mut pixels = vec![0u8; (w * h * 4) as usize];
            GetDIBits(
                hdc_mem,
                hbm,
                0,
                h as u32,
                Some(pixels.as_mut_ptr() as _),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            // BGRA -> RGBA
            for chunk in pixels.chunks_exact_mut(4) {
                chunk.swap(0, 2);
            }

            SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbm.into());
            DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);

            pixels
        }
    }
}

#[cfg(target_os = "windows")]
mod game_window {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, EndPaint, FillRect, PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub struct GameWindow {
        hwnd: HWND,
    }

    impl GameWindow {
        pub fn position(&self) -> (i32, i32) {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetWindowRect(self.hwnd, &mut rect);
            }
            (rect.left, rect.top)
        }

        pub fn size(&self) -> (i32, i32) {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetWindowRect(self.hwnd, &mut rect);
            }
            (rect.right - rect.left, rect.bottom - rect.top)
        }

        pub fn set_foreground(&self) {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
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
            hbrBackground: unsafe {
                CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x1E1E1E))
            },
            ..Default::default()
        };

        unsafe { RegisterClassExW(&wc) };

        let title_wide = wide(title);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                w,
                h,
                None,
                None,
                Some(hinstance.into()),
                None,
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
