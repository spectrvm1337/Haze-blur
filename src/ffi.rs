//! A C interface, built into `haze.dll` with `--features ffi`, for C#, Python (ctypes), Electron
//! addons and anything else that can call a DLL. Attach on the window's thread; the other calls
//! work from any thread.
//!
//! ```c
//! void*   haze_attach(void* hwnd, float radius, float strength, float tint, uint32_t tint_rgb, float saturation);
//! int32_t haze_last_error(void);  /* HRESULT of the last haze_attach that returned NULL on this thread */
//! void    haze_set_style(void* haze, float radius, float strength, float tint, uint32_t tint_rgb, float saturation, uint32_t millis);
//! void    haze_set_corner(void* haze, float k);  /* CSS superellipse(k): 1 round, 2 squircle */
//! void    haze_fade(void* haze, int visible, uint32_t millis);
//! void    haze_set_regions(void* haze, const float* rects, uint32_t count);     /* count * [x, y, w, h, radius] */
//! void    haze_set_regions_ex(void* haze, const float* rects, uint32_t count);  /* count * [x, y, w, h, radius, opacity, k] */
//! void    haze_set_scale(void* haze, float scale);  /* <= 0 follows the window's DPI */
//! int     haze_is_attached(void* haze);
//! void    haze_detach(void* haze);  /* takes the blur off and frees the handle */
//! int     haze_supported(void);
//! ```
//! `tint_rgb` is `0xRRGGBB`. `millis` of 0 moves at once. In the regions a `radius` below 0 and a `k`
//! of 0 or below take the style's. A `rects` of NULL blurs the whole window; a count of 0 blurs nothing.
use std::cell::Cell;
use std::ffi::c_void;
use std::time::Duration;

use crate::{Corner, Haze, Region, Style};

thread_local! {
    static LAST_ERROR: Cell<i32> = const { Cell::new(0) };
}

fn style(base: Style, radius: f32, strength: f32, tint: f32, tint_rgb: u32, saturation: f32) -> Style {
    base.radius(radius)
        .strength(strength)
        .tint(tint)
        .tint_color([(tint_rgb >> 16) as u8, (tint_rgb >> 8) as u8, tint_rgb as u8])
        .saturation(saturation)
}

/// Returns a handle, or null if the blur could not be started; `haze_last_error` says why.
#[no_mangle]
pub extern "C" fn haze_attach(hwnd: *mut c_void, radius: f32, strength: f32, tint: f32, tint_rgb: u32, saturation: f32) -> *mut Haze {
    match Haze::attach_hwnd(hwnd, style(Style::default(), radius, strength, tint, tint_rgb, saturation)) {
        Ok(h) => {
            LAST_ERROR.with(|e| e.set(0));
            Box::into_raw(Box::new(h))
        }
        Err(err) => {
            LAST_ERROR.with(|e| e.set(err.code().0));
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn haze_last_error() -> i32 {
    LAST_ERROR.with(|e| e.get())
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_set_style(
    haze: *const Haze,
    radius: f32,
    strength: f32,
    tint: f32,
    tint_rgb: u32,
    saturation: f32,
    millis: u32,
) {
    if let Some(h) = haze.as_ref() {
        h.animate_to(style(h.style(), radius, strength, tint, tint_rgb, saturation), Duration::from_millis(millis as u64));
    }
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_set_corner(haze: *const Haze, k: f32) {
    if let Some(h) = haze.as_ref() {
        let style = h.style();
        let corner = Corner::Superellipse(k);
        // Only a new shape is set: setting the style at once would cut a running animation short.
        if style.corner(corner).corner.k() != style.corner.k() {
            h.set_style(style.corner(corner));
        }
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

unsafe fn regions(haze: *const Haze, rects: *const f32, count: u32, stride: usize) {
    let Some(h) = haze.as_ref() else { return };
    if rects.is_null() {
        return h.cover_window();
    }
    let regions: Vec<Region> = std::slice::from_raw_parts(rects, count as usize * stride)
        .chunks_exact(stride)
        .map(|r| {
            let mut region = Region::new(r[0], r[1], r[2], r[3]);
            if r[4] >= 0.0 {
                region = region.radius(r[4]);
            }
            if stride == 7 {
                region = region.opacity(r[5]);
                if r[6] > 0.0 {
                    region = region.corner(Corner::Superellipse(r[6]));
                }
            }
            region
        })
        .collect();
    h.set_regions(&regions);
}

/// # Safety
/// `haze` is a handle from `haze_attach`, and `rects` is null or points to `count` groups of five floats.
#[no_mangle]
pub unsafe extern "C" fn haze_set_regions(haze: *const Haze, rects: *const f32, count: u32) {
    regions(haze, rects, count, 5);
}

/// # Safety
/// `haze` is a handle from `haze_attach`, and `rects` is null or points to `count` groups of seven floats.
#[no_mangle]
pub unsafe extern "C" fn haze_set_regions_ex(haze: *const Haze, rects: *const f32, count: u32) {
    regions(haze, rects, count, 7);
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_set_scale(haze: *const Haze, scale: f32) {
    if let Some(h) = haze.as_ref() {
        h.set_scale((scale > 0.0).then_some(scale));
    }
}

/// # Safety
/// `haze` is a handle from `haze_attach` that has not been detached.
#[no_mangle]
pub unsafe extern "C" fn haze_is_attached(haze: *const Haze) -> i32 {
    haze.as_ref().is_some_and(Haze::is_attached) as i32
}

/// # Safety
/// `haze` is a handle from `haze_attach`; it is freed here and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn haze_detach(haze: *mut Haze) {
    if !haze.is_null() {
        Box::from_raw(haze).detach();
    }
}

#[no_mangle]
pub extern "C" fn haze_supported() -> i32 {
    Haze::supported() as i32
}
