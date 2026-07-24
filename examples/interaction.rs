#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("this example only runs on windows");
}

#[cfg(target_os = "windows")]
fn main() -> procmod_overlay::Result<()> {
    use procmod_overlay::{
        Color, InputEvent, InteractionMode, MouseButton, Overlay, OverlayTarget,
    };
    use std::io::Write;
    use std::time::{Duration, Instant};

    let game = game_window::create("Procmod Interaction Target", 640, 480);
    std::thread::sleep(Duration::from_millis(250));
    game.set_foreground();
    let mut overlay = Overlay::new(OverlayTarget::Title("Procmod Interaction Target".into()))?;
    overlay.set_interaction_mode(InteractionMode::Interactive)?;
    let mut log = std::fs::File::create("interaction-events.txt").unwrap();
    let started = Instant::now();
    let mut running = true;
    while running && started.elapsed() < Duration::from_secs(20) {
        overlay.begin_frame()?;
        overlay.rect_filled(160.0, 140.0, 320.0, 180.0, Color::rgba(20, 25, 35, 235));
        overlay.rect(160.0, 140.0, 320.0, 180.0, Color::CYAN);
        overlay.text(190.0, 180.0, "Interactive overlay", 22.0, Color::WHITE);
        overlay.text(
            190.0,
            220.0,
            "Click here, then press Escape",
            16.0,
            Color::CYAN,
        );
        overlay.end_frame()?;

        for event in overlay.drain_input_events() {
            writeln!(log, "{event:?}").unwrap();
            match event {
                InputEvent::MouseButton {
                    button: MouseButton::Left,
                    pressed: false,
                    ..
                } => writeln!(log, "CLICK_CONFIRMED").unwrap(),
                InputEvent::Key {
                    virtual_key: 0x1b, ..
                } => {
                    overlay.set_interaction_mode(InteractionMode::PassThrough)?;
                    writeln!(log, "PASS_THROUGH_RESTORED").unwrap();
                    running = false;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    drop(overlay);
    writeln!(log, "TARGET_FOREGROUND={}", game.is_foreground()).unwrap();
    Ok(())
}

#[cfg(target_os = "windows")]
mod game_window {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub struct GameWindow {
        hwnd: HWND,
    }

    impl GameWindow {
        pub fn set_foreground(&self) {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }

        pub fn is_foreground(&self) -> bool {
            unsafe { GetForegroundWindow() == self.hwnd }
        }
    }

    impl Drop for GameWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    pub fn create(title: &str, width: i32, height: i32) -> GameWindow {
        let class_name = wide(title);
        let hinstance = unsafe { GetModuleHandleW(None).unwrap() };
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        unsafe { RegisterClassExW(&class) };
        let title = wide(title);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                100,
                100,
                width,
                height,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .unwrap();
        GameWindow { hwnd }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }
}
