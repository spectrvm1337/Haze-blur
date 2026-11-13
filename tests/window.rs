//! Runs the blur on real windows. Each test owns a window on its own thread and pumps its messages,
//! since calls from other threads reach the window through its message queue. The windows are made
//! off screen, so nothing shows while the tests run.
#![cfg(windows)]

use std::time::{Duration, Instant};

use haze::{Corner, Easing, Haze, Region, Style, Transition};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn plain_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn window_with(ex: WINDOW_EX_STYLE) -> HWND {
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
            ex | WS_EX_TOOLWINDOW,
            w!("HazeTest"),
            w!("test"),
            WS_POPUP,
            -20000,
            -20000,
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

fn window() -> HWND {
    window_with(WS_EX_NOREDIRECTIONBITMAP)
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

/// Nothing the compositor was asked to do failed.
fn clean(haze: &Haze) {
    if let Some(e) = haze.last_error() {
        panic!("the compositor refused a call: {e}");
    }
}

#[test]
fn destroying_the_window_with_the_blur_on_is_clean() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.animate_to(Style::light(), Duration::from_millis(500));
    pump(50);
    clean(&haze);
    destroy(hwnd); // with an animation still running
    assert!(!haze.is_attached());
    // A handle that outlives its window does nothing.
    haze.set_style(Style::frost());
    haze.set_regions(&[Region::new(0.0, 0.0, 10.0, 10.0)]);
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
fn attaching_from_another_thread_or_to_no_window_is_an_error() {
    let hwnd = window();
    let raw = hwnd.0 as isize;
    let result = std::thread::spawn(move || Haze::attach_hwnd(raw as *mut _, Style::dark()).is_err()).join().unwrap();
    assert!(result);
    destroy(hwnd);
    assert!(Haze::attach_hwnd(hwnd.0, Style::dark()).is_err());
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
        // A burst of calls, as a page sending its cards every frame.
        for i in 0..200 {
            other.set_regions(&[Region::new(10.0, i as f32, 100.0, 50.0).radius(25.0)]);
        }
    });
    pump(200);
    thread.join().unwrap();
    assert_eq!(haze.style(), target);
    clean(&haze);
    destroy(hwnd);
}

#[test]
fn detach_gives_the_window_back() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.detach();
    pump(30);
    assert!(!haze.is_attached());
    // Nothing is left behind, so the blur can go on again.
    let again = Haze::attach_hwnd(hwnd.0, Style::light()).unwrap();
    assert!(again.is_attached());
    // The old handle does not drive the new blur.
    haze.set_style(Style::frost());
    pump(20);
    assert_eq!(again.style(), Style::light());
    destroy(hwnd);
}

static mut BEFORE: isize = 0;

unsafe extern "system" fn wrapping_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let prev: WNDPROC = std::mem::transmute(BEFORE);
    CallWindowProcW(prev, hwnd, msg, wp, lp)
}

#[test]
fn detach_and_attach_again_with_another_subclass_on_top() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    // Other code replaces the window procedure after the blur went on, the old-fashioned way.
    unsafe { BEFORE = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wrapping_proc as *const () as isize) };
    haze.detach();
    pump(30);
    let again = Haze::attach_hwnd(hwnd.0, Style::light()).expect("attach after a detach under another subclass");
    unsafe {
        let _ = SetWindowPos(hwnd, None, 0, 0, 400, 300, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    pump(30);
    clean(&again);
    destroy(hwnd);
}

#[test]
fn regions_update_in_place_and_clear() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.set_regions(&[Region::new(0.0, 0.0, 80.0, 200.0), Region::new(100.0, 20.0, 180.0, 160.0).radius(16.0)]);
    haze.animate_to(Style::light().radius(30.0), Duration::from_millis(80));
    // Moved halfway through the animation: the shapes follow without starting over.
    haze.set_regions(&[Region::new(4.0, 0.0, 80.0, 200.0), Region::new(100.0, 24.0, 180.0, 160.0).opacity(0.5)]);
    pump(40);
    haze.set_regions(&[Region::new(0.0, 0.0, 50.0, 50.0)]);
    haze.set_regions(&[]); // nothing
    haze.set_regions(&[Region::new(f32::NAN, 0.0, 10.0, 10.0), Region::new(0.0, 0.0, -5.0, 10.0)]); // nothing drawable
    pump(60);
    haze.cover_window();
    pump(30);
    clean(&haze);
    destroy(hwnd);
}

#[test]
fn squircles_and_round_corners_switch_and_animate() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark().corner(Corner::Squircle)).unwrap();
    haze.animate_to(Style::dark().corner(Corner::Squircle).radius(40.0), Duration::from_millis(60));
    pump(20);
    haze.set_regions(&[
        Region::new(10.0, 10.0, 120.0, 60.0),
        Region::new(10.0, 80.0, 120.0, 60.0).corner(Corner::Round).radius(12.0),
        Region::new(140.0, 10.0, 120.0, 60.0).corner(Corner::Superellipse(3.0)),
    ]);
    haze.animate_to(Style::dark().radius(10.0), Duration::from_millis(60)); // back to round
    pump(100);
    haze.set_style(Style::dark().corner(Corner::Superellipse(f32::NAN)));
    pump(20);
    clean(&haze);
    destroy(hwnd);
}

#[test]
fn every_effect_comes_and_goes() {
    let hwnd = window();
    let plain = Style::default().strength(0.0);
    let haze = Haze::attach_hwnd(hwnd.0, plain).unwrap(); // the backdrop as it is, no effect
    let full = Style::frost().tint(0.5).saturation(1.6);
    let t = Transition::new(Duration::from_millis(40)).delay(Duration::from_millis(10)).easing(Easing::Linear);
    haze.animate_to(full, t);
    pump(250); // through the settle timer
    haze.animate_to(plain, Duration::from_millis(40));
    pump(250);
    haze.set_style(Style::dark());
    haze.set_style(plain);
    clean(&haze);
    destroy(hwnd);
}

#[test]
fn hiding_resizing_and_scaling_keep_working() {
    let hwnd = window();
    let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
    haze.set_regions(&[Region::new(10.0, 10.0, 100.0, 50.0)]);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        pump(30);
        let _ = SetWindowPos(hwnd, None, 0, 0, 500, 400, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        pump(30);
        haze.set_scale(Some(1.0));
        haze.set_scale(Some(2.25));
        haze.set_scale(Some(f32::NAN)); // the window's DPI again
        haze.cover_window();
        pump(30);
        let _ = ShowWindow(hwnd, SW_HIDE);
        pump(30);
    }
    clean(&haze);
    destroy(hwnd);
}

#[test]
fn the_blur_comes_back_after_dwm_restarts() {
    for ex in [WS_EX_NOREDIRECTIONBITMAP, WINDOW_EX_STYLE(0)] {
        let hwnd = window_with(ex);
        let haze = Haze::attach_hwnd(hwnd.0, Style::dark()).unwrap();
        haze.set_regions(&[Region::new(10.0, 10.0, 100.0, 50.0).corner(Corner::Squircle)]);
        haze.fade(false, Transition::INSTANT);
        unsafe { SendMessageW(hwnd, WM_DWMCOMPOSITIONCHANGED, None, None) };
        pump(30);
        assert!(haze.is_attached());
        haze.animate_to(Style::light(), Duration::from_millis(30));
        pump(60);
        clean(&haze);
        destroy(hwnd);
    }
}

#[cfg(feature = "ffi")]
#[test]
fn the_c_interface_runs() {
    use haze::ffi::*;
    assert_eq!(haze_supported(), 1);
    assert!(haze_attach(std::ptr::null_mut(), 20.0, 12.0, 0.4, 0x0A0A0E, 1.0).is_null());
    assert_ne!(haze_last_error(), 0);
    let hwnd = window();
    let h = haze_attach(hwnd.0, 20.0, 12.0, 0.4, 0x0A0A0E, 1.0);
    assert!(!h.is_null());
    assert_eq!(haze_last_error(), 0);
    let rects = [0.0f32, 0.0, 100.0, 100.0, -1.0, 120.0, 0.0, 100.0, 100.0, 12.0];
    let rects_ex = [0.0f32, 0.0, 100.0, 100.0, -1.0, 0.5, 2.0];
    unsafe {
        haze_set_style(h, 30.0, 20.0, 0.5, 0xFFFFFF, 1.2, 40);
        haze_set_corner(h, 2.0);
        haze_fade(h, 0, 40);
        haze_set_regions(h, rects.as_ptr(), 2);
        haze_set_regions_ex(h, rects_ex.as_ptr(), 1);
        haze_set_scale(h, 1.0);
        pump(80);
        assert_eq!(haze_is_attached(h), 1);
        haze_set_regions(h, std::ptr::null(), 0);
        haze_detach(h);
    }
    pump(30);
    destroy(hwnd);
}
