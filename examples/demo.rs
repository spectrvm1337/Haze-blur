//! A bare Win32 window with nothing in it but the blur. Drag it by the body, Space fades the blur out
//! and in, Esc closes. `cargo run --example demo -- --seconds 3` closes itself after 3 s.
use std::cell::{Cell, RefCell};
use std::time::Duration;

use haze::{Haze, Style};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_SPACE};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static HAZE: RefCell<Option<Haze>> = const { RefCell::new(None) };
    static SHOWN: Cell<bool> = const { Cell::new(true) };
}

fn redraw(hwnd: HWND) {
    let mut r = Default::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        HAZE.with(|h| {
            if let Some(h) = h.borrow().as_ref() {
                if let Err(e) = h.resize((r.right - r.left) as f32, (r.bottom - r.top) as f32, scale) {
                    eprintln!("resize: {e}");
                }
            }
        });
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            redraw(hwnd);
            LRESULT(0)
        }
        // the whole window is the title bar, so it drags by the body
        WM_NCHITTEST => LRESULT(HTCAPTION as isize),
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_SPACE.0 as usize => {
            let shown = !SHOWN.get();
            SHOWN.set(shown);
            HAZE.with(|h| {
                if let Some(h) = h.borrow().as_ref() {
                    let _ = h.fade(shown, Duration::from_millis(300));
                }
            });
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            HAZE.with(|h| h.borrow_mut().take()); // drops the blur before the window goes
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() -> windows::core::Result<()> {
    let seconds: Option<u32> = std::env::args().skip_while(|a| a != "--seconds").nth(1).and_then(|s| s.parse().ok());

    unsafe {
        let _ = SetProcessDPIAware();
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("HazeDemo"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        RegisterClassW(&class);

        // No redirection bitmap: the window shows only what Composition draws into it.
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("HazeDemo"),
            w!("Haze"),
            WS_POPUP | WS_VISIBLE,
            200,
            200,
            480,
            320,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let haze = Haze::new(hwnd.0, Style::default())?;
        HAZE.with(|h| *h.borrow_mut() = Some(haze));
        redraw(hwnd);
        println!("haze is up on the window; Space fades it, Esc closes");

        if let Some(s) = seconds {
            SetTimer(Some(hwnd), 1, s * 1000, None);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
