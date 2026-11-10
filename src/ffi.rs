//! A C interface, built into `haze.dll` with `--features ffi`, for C#, Python (ctypes), Electron
//! addons and anything else that can call a DLL. Call it on the window's thread.
//!
//! ```c
//! void* haze_attach(void* hwnd, float radius, float strength, float tint, uint32_t tint_rgb, float saturation);
//! void  haze_set_style(void* haze, float radius, float strength, float tint, uint32_t tint_rgb, float saturation, uint32_t millis);
//! void  haze_fade(void* haze, int visible, uint32_t millis);
//! void  haze_set_regions(void* haze, const float* rects, uint32_t count); /* count * [x, y, w, h, radius]; radius < 0 uses the style's */
//! void  haze_detach(void* haze);   /* takes the blur off and frees the handle */
//! int   haze_supported(void);
//! ```
//! `tint_rgb` is `0xRRGGBB`. `millis` of 0 moves at once.
use std::ffi::c_void;
use std::time::Duration;

use crate::{Haze, Region, Style};

fn style(radius: f32, strength: f32, tint: f32, tint_rgb: u32, saturation: f32) -> Style {
    Style::default().radius(radius).strength(strength).tint(tint).tint_color([
        (tint_rgb >> 16) as u8,
        (tint_rgb >> 8) as u8,
        tint_rgb as u8,
    ]).saturation(saturation)
}

/// Returns a handle, or null if the blur could not be started.
#[no_mangle]
pub extern "C" fn haze_attach(hwnd: *mut c_void, radius: f32, strength: f32, tint: f32, tint_rgb: u32, saturation: f32) -> *mut Haze {
    match Haze::attach_hwnd(hwnd, style(radius, strength, tint, tint_rgb, saturation)) {
        Ok(h) => Box::into_raw(Box::new(h)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_set_style(haze: *const Haze, radius: f32, strength: f32, tint: f32, tint_rgb: u32, saturation: f32, millis: u32) {
    if let Some(h) = haze.as_ref() {
        h.animate_to(style(radius, strength, tint, tint_rgb, saturation), Duration::from_millis(millis as u64));
    }
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_fade(haze: *const Haze, visible: i32, millis: u32) {
    if let Some(h) = haze.as_ref() {
        h.fade(visible != 0, Duration::from_millis(millis as u64));
    }
}

/// # Safety
/// `haze` is a handle from `haze_attach`, and `rects` points to `count` groups of five floats.
#[no_mangle]
pub unsafe extern "C" fn haze_set_regions(haze: *const Haze, rects: *const f32, count: u32) {
    let Some(h) = haze.as_ref() else { return };
    let regions: Vec<Region> = if rects.is_null() || count == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(rects, count as usize * 5)
            .chunks_exact(5)
            .map(|r| {
                let region = Region::new(r[0], r[1], r[2], r[3]);
                if r[4] >= 0.0 { region.radius(r[4]) } else { region }
            })
            .collect()
    };
    h.set_regions(&regions);
}

/// # Safety
/// `haze` is a handle from `haze_attach`; it is freed here and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn haze_detach(haze: *mut Haze) {
    if !haze.is_null() {
        let h = Box::from_raw(haze);
        h.detach();
    }
}

#[no_mangle]
pub extern "C" fn haze_supported() -> i32 {
    Haze::supported() as i32
}
