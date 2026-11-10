//! Runs the blur on real (hidden) windows. Each test owns a window on its own thread and pumps its
//! messages, since calls from other threads reach the window through its message queue.
#![cfg(windows)]

use std::time::{Duration, Instant};

use haze::{Haze, Region, Style};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn plain_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn window() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        // Registered once per process; later calls fail harmlessly.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(plain_proc),
            hInstance: instance.into(),
            lpszClassName: w!("HazeTest"),
            ..Default::default()
        });
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("HazeTest"),
            w!("test"),
            WS_POPUP,
            0,
            0,
            300,
            200,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap()
    }
}

fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < until {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn destroy(hwnd: HWND) {
    unsafe { DestroyWindow(hwnd).unwrap() };
    pump(30);
}

#[test]
fn destroying_the_window_with_the_blur_on_is_clean() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.animate_to(Style::light(), Duration::from_millis(500));
    pump(50);
    destroy(hwnd); // with an animation still running
    // A handle that outlives its window does nothing.
    haze.set_style(Style::frost());
    haze.detach();
}

#[test]
fn attaching_twice_is_an_error() {
    let hwnd = window();
    let _haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    assert!(Haze::attach_hwnd(hwnd.0, Style::dark()).is_err());
    destroy(hwnd);
}

#[test]
fn attaching_from_another_thread_is_an_error() {
    let hwnd = window();
    let raw = hwnd.0 as isize;
    let result = std::thread::spawn(move || Haze::attach_hwnd(raw as *mut _, Style::dark()).is_err()).join().unwrap();
    assert!(result);
    destroy(hwnd);
}

#[test]
fn a_handle_works_from_another_thread() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    let target = Style::frost().radius(33.0);
    let other = haze.clone();
    let thread = std::thread::spawn(move || {
        other.animate_to(target, Duration::from_millis(50));
        other.fade(false, Duration::from_millis(50));
        other.set_regions(&[Region::new(10.0, 10.0, 100.0, 50.0).radius(25.0)]);
    });
    pump(200);
    thread.join().unwrap();
    assert_eq!(haze.style(), target);
    destroy(hwnd);
}

#[test]
fn detach_gives_the_window_back() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.detach();
    pump(30);
    // Nothing is left behind, so the blur can go on again.
    let again = Haze::attach_hwnd(hwnd.0, Style::light());
    assert!(again.is_ok());
    destroy(hwnd);
}

#[test]
fn regions_can_be_set_and_cleared() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.set_regions(&[Region::new(0.0, 0.0, 80.0, 200.0), Region::new(100.0, 20.0, 180.0, 160.0).radius(16.0)]);
    haze.animate_to(Style::light(), Duration::from_millis(50));
    pump(100);
    haze.set_regions(&[]);
    pump(30);
    destroy(hwnd);
}

#[test]
fn hiding_and_resizing_keep_working() {
    let hwnd = window();
    let _haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        pump(30);
        let _ = SetWindowPos(hwnd, None, 0, 0, 500, 400, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        pump(30);
        let _ = ShowWindow(hwnd, SW_HIDE);
        pump(30);
    }
    destroy(hwnd);
}

#[cfg(feature = "ffi")]
#[test]
fn the_c_interface_runs() {
    use haze::ffi::*;
    assert_eq!(haze_supported(), 1);
    let hwnd = window();
    let h = haze_attach(hwnd.0, 20.0, 12.0, 0.4, 0x0A0A0E, 1.0);
    assert!(!h.is_null());
    let rects = [0.0f32, 0.0, 100.0, 100.0, -1.0, 120.0, 0.0, 100.0, 100.0, 12.0];
    unsafe {
        haze_set_style(h, 30.0, 20.0, 0.5, 0xFFFFFF, 1.2, 40);
        haze_fade(h, 0, 40);
        haze_set_regions(h, rects.as_ptr(), 2);
        pump(80);
        haze_set_regions(h, std::ptr::null(), 0);
        haze_detach(h);
    }
    pump(30);
    destroy(hwnd);
}
