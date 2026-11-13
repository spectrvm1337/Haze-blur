//! A bare Win32 window with nothing in it but the blur, which changes style on its own every couple
//! of seconds (the compositor does the in-between frames; the app only says where to go next).
//! Drag it by the body, Space fades the blur out and in, `D` switches between dark and light, `S`
//! between round and squircle corners, Esc closes.
//! `--static` stops the changes, `--seconds 3` closes the window by itself after 3 s.
use std::cell::{Cell, RefCell};
use std::time::Duration;

use haze::{Corner, Haze, Style};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_SPACE};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static HAZE: RefCell<Option<Haze>> = const { RefCell::new(None) };
    static SHOWN: Cell<bool> = const { Cell::new(true) };
    static LIGHT: Cell<bool> = const { Cell::new(false) };
    static SQUIRCLE: Cell<bool> = const { Cell::new(false) };
    static STEP: Cell<usize> = const { Cell::new(0) };
}

const TIMER_CLOSE: usize = 1;
const TIMER_NEXT: usize = 2;
const EVERY_MS: u32 = 2400;

/// Where the blur goes next: (radius, strength, tint).
const STEPS: [(f32, f32, f32); 4] = [(70.0, 22.0, 0.55), (10.0, 8.0, 0.35), (44.0, 28.0, 0.5), (96.0, 14.0, 0.62)];

fn base() -> Style {
    let style = if LIGHT.get() { Style::light() } else { Style::dark() };
    style.corner(if SQUIRCLE.get() { Corner::Squircle } else { Corner::Round })
}

fn next_style() {
    let (radius, strength, tint) = STEPS[STEP.get() % STEPS.len()];
    STEP.set(STEP.get() + 1);
    HAZE.with(|h| {
        if let Some(h) = h.borrow().as_ref() {
            h.animate_to(base().radius(radius).strength(strength).tint(tint), Duration::from_millis(1800));
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // the whole window is the title bar, so it drags by the body
        WM_NCHITTEST => LRESULT(HTCAPTION as isize),
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_SPACE.0 as usize => {
            let shown = !SHOWN.get();
            SHOWN.set(shown);
            HAZE.with(|h| h.borrow().as_ref().map(|h| h.fade(shown, Duration::from_millis(300))));
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == 'D' as usize => {
            LIGHT.set(!LIGHT.get());
            HAZE.with(|h| h.borrow().as_ref().map(|h| h.animate_to(base(), Duration::from_millis(500))));
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == 'S' as usize => {
            SQUIRCLE.set(!SQUIRCLE.get());
            HAZE.with(|h| h.borrow().as_ref().map(|h| h.set_style(h.style().corner(base().corner))));
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_NEXT => {
            next_style();
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: Option<u32> = args.iter().position(|a| a == "--seconds").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok());

    if !Haze::supported() {
        eprintln!("this Windows cannot show the blur (Windows 10 1809 or newer is needed)");
        return Ok(());
    }

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
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST,
            w!("HazeDemo"),
            w!("Haze"),
            WS_POPUP | WS_VISIBLE,
            200,
            200,
            520,
            340,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // One line. The blur follows the window's size and DPI from here on.
        let haze = Haze::attach_hwnd(hwnd.0, Style::dark().radius(40.0).strength(16.0))?;
        HAZE.with(|h| *h.borrow_mut() = Some(haze));
        println!("haze is up; Space fades it, D switches dark and light, S round and squircle, Esc closes");

        if let Some(s) = seconds {
            SetTimer(Some(hwnd), TIMER_CLOSE, s * 1000, None);
        }
        if !args.iter().any(|a| a == "--static") {
            SetTimer(Some(hwnd), TIMER_NEXT, EVERY_MS, None);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
