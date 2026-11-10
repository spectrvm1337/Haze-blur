//! **Haze**: a rounded-corner backdrop blur for Windows 10 windows.
//!
//! Windows 10 can only blur a whole rectangle behind a window (the acrylic and blur accents), so a
//! transparent window with a rounded card in it gets a blur with square corners sticking out. Haze
//! draws the blur itself with `Windows.UI.Composition`: one sprite painted with the `HostBackdropBrush`
//! (what is behind the window), a Gaussian blur on top of the system's own, a dark layer over it, and
//! a rounded-rectangle clip that gives them their shape. They sit under the window's content, so a
//! transparent webview or a layered child window draws over them.
//!
//! ```no_run
//! # fn demo(window: &impl raw_window_handle::HasWindowHandle) -> windows::core::Result<()> {
//! use std::time::Duration;
//! use haze::{Haze, Style};
//!
//! // On the thread that owns the window. After this the blur follows the window's size and DPI by itself.
//! let haze = Haze::attach(window, Style::dark().radius(20.0))?;
//!
//! // From any thread, later. Changes are animated by the compositor, nothing is called per frame.
//! haze.animate_to(Style::light().radius(32.0), Duration::from_millis(300));
//! haze.fade(false, Duration::from_millis(200)); // fade it with your own fade
//! # Ok(()) }
//! ```
//!
//! The blur belongs to the window: it goes away with it, or on [`Haze::detach`]. The handle can be
//! cloned and sent to other threads.
//!
//! It uses `SetWindowCompositionAttribute`, which is undocumented, to let the brush see behind the
//! window (accent state 5, `ACCENT_ENABLE_HOSTBACKDROP`).
#![cfg(windows)]

mod effect;
mod engine;
#[cfg(feature = "ffi")]
pub mod ffi;

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetPropW, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow, PostMessageW, RemovePropW,
    SetPropW, SetWindowLongPtrW, GWLP_WNDPROC, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_NCDESTROY, WM_SHOWWINDOW, WM_SIZE,
};

pub use windows::core::{Error, Result};

use engine::Engine;

/// How the blur looks. Build one from a preset and change what you need:
/// `Style::dark().radius(24.0).strength(18.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Style {
    /// Corner radius in device-independent pixels (CSS px); it is scaled by the window's DPI.
    pub radius: f32,
    /// Extra Gaussian blur on top of the system's own, in px.
    pub strength: f32,
    /// How much of the tint colour is laid over the blur, 0.0 (none) to 1.0 (solid). A blur over a
    /// light or busy backdrop is hard to see; tinting it makes the shape stand out.
    pub tint: f32,
    /// The tint's colour, RGB.
    pub tint_color: [u8; 3],
    /// Colour saturation of what shows through: 0 is grey, 1 leaves the colours alone, above 1
    /// strengthens them.
    pub saturation: f32,
}

impl Default for Style {
    /// Plain blur, no tint.
    fn default() -> Self {
        Style { radius: 20.0, strength: 10.0, tint: 0.0, tint_color: [0, 0, 0], saturation: 1.0 }
    }
}

impl Style {
    /// Dark glass: a graphite tint, easy to read light text on.
    pub fn dark() -> Style {
        Style { radius: 20.0, strength: 14.0, tint: 0.45, tint_color: [10, 10, 14], saturation: 1.0 }
    }
    /// Light glass.
    pub fn light() -> Style {
        Style { radius: 20.0, strength: 14.0, tint: 0.4, tint_color: [244, 245, 250], saturation: 1.0 }
    }
    /// Frosted: a strong blur with only a trace of white.
    pub fn frost() -> Style {
        Style { radius: 20.0, strength: 24.0, tint: 0.16, tint_color: [255, 255, 255], saturation: 1.25 }
    }
    /// Corner radius in CSS px.
    pub fn radius(mut self, radius: f32) -> Style {
        self.radius = radius.max(0.0);
        self
    }
    /// Blur strength in px.
    pub fn strength(mut self, strength: f32) -> Style {
        self.strength = strength.max(0.0);
        self
    }
    /// Tint opacity, 0.0 to 1.0.
    pub fn tint(mut self, tint: f32) -> Style {
        self.tint = tint.clamp(0.0, 1.0);
        self
    }
    /// Tint colour.
    pub fn tint_color(mut self, rgb: [u8; 3]) -> Style {
        self.tint_color = rgb;
        self
    }
    /// Colour saturation of what shows through; 1 leaves it alone.
    pub fn saturation(mut self, saturation: f32) -> Style {
        self.saturation = saturation.max(0.0);
        self
    }
}

/// A part of the window to blur, in CSS px from the client area's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// This region's own corner radius, instead of the style's. It is not animated by
    /// [`Haze::animate_to`].
    pub radius: Option<f32>,
}

impl Region {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Region {
        Region { x, y, width, height, radius: None }
    }
    /// Gives this region its own corner radius. Half of its height makes a pill.
    pub fn radius(mut self, radius: f32) -> Region {
        self.radius = Some(radius.max(0.0));
        self
    }
}

enum Cmd {
    Style(Style, Duration),
    Fade(bool, Duration),
    Regions(Vec<Region>),
    Detach,
}

/// What the handles and the window's engine share.
pub(crate) struct Shared {
    queue: Mutex<Vec<Cmd>>,
    style: Mutex<Style>,
}

/// A rounded blur under one window. Cheap to clone and safe to send to other threads; the
/// calls are carried out on the window's own thread.
#[derive(Clone)]
pub struct Haze {
    hwnd: isize,
    shared: Arc<Shared>,
}

const WM_HAZE: u32 = WM_APP + 0x4A5;
/// The window property that holds the [`Slot`].
const PROP: PCWSTR = w!("HazeSlot");

/// What is kept on the window: the procedure it had, and the engine. The engine is gone after a
/// detach; the slot stays if somebody else replaced the procedure after us and it cannot be undone.
struct Slot {
    prev: isize,
    engine: Option<Engine>,
}

impl Haze {
    /// Whether this Windows has what the blur needs (Windows 10 1803 or newer).
    pub fn supported() -> bool {
        windows::Foundation::Metadata::ApiInformation::IsMethodPresent(
            &HSTRING::from("Windows.UI.Composition.Compositor"),
            &HSTRING::from("CreateHostBackdropBrush"),
        )
        .unwrap_or(false)
    }

    /// Puts the blur under `window` (a Tauri window, a winit window, anything with a window handle).
    /// Call it on the thread that owns the window, and make the window transparent so that the blur
    /// shows. The blur follows the window's size and DPI from then on.
    #[cfg(feature = "raw-window-handle")]
    pub fn attach(window: &impl raw_window_handle::HasWindowHandle, style: Style) -> Result<Haze> {
        use raw_window_handle::RawWindowHandle;
        match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Haze::attach_hwnd(h.hwnd.get() as *mut c_void, style),
            _ => Err(Error::new(windows::Win32::Foundation::E_INVALIDARG, "not a Win32 window")),
        }
    }

    /// Like [`attach`](Self::attach), from a raw `HWND`.
    pub fn attach_hwnd(hwnd: *mut c_void, style: Style) -> Result<Haze> {
        use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, RPC_E_WRONG_THREAD};
        let hwnd = HWND(hwnd);
        if !on_window_thread(hwnd) {
            return Err(Error::new(RPC_E_WRONG_THREAD, "attach has to run on the window's thread"));
        }
        if unsafe { slot_of(hwnd) }.is_some() {
            return Err(ERROR_ALREADY_EXISTS.to_hresult().into());
        }

        let shared = Arc::new(Shared { queue: Mutex::new(Vec::new()), style: Mutex::new(style) });
        let engine = Engine::new(hwnd, style, shared.clone())?;

        // The window procedure is replaced by hand: SetWindowSubclass would need comctl32 v6, which an
        // exe only gets with a manifest, and a library cannot add one to the program that uses it.
        let slot = Box::into_raw(Box::new(Slot { prev: 0, engine: Some(engine) }));
        unsafe {
            if let Err(e) = SetPropW(hwnd, PROP, Some(HANDLE(slot as *mut c_void))) {
                drop(Box::from_raw(slot));
                return Err(e);
            }
            (*slot).prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, subclass_proc as *const () as usize as isize);
        }
        Ok(Haze { hwnd: hwnd.0 as isize, shared })
    }

    /// The style that was asked for last.
    pub fn style(&self) -> Style {
        *self.shared.style.lock().unwrap()
    }

    /// Moves to `style` at once.
    pub fn set_style(&self, style: Style) {
        self.send(Cmd::Style(style, Duration::ZERO));
    }

    /// Moves to `style` over `duration`, on the page's usual curve `cubic-bezier(0.32, 0.72, 0, 1)`.
    /// The radius, the strength, the tint and its colour all move together.
    pub fn animate_to(&self, style: Style, duration: Duration) {
        self.send(Cmd::Style(style, duration));
    }

    /// Fades the blur in or out over `duration`, or at once if it is zero. A page that fades its own
    /// content leaves the blur at full strength unless it is faded with it.
    pub fn fade(&self, visible: bool, duration: Duration) {
        self.send(Cmd::Fade(visible, duration));
    }

    /// Blurs only these parts of the window, for example a sidebar and a card. An empty list goes
    /// back to the whole window. The regions stay where they are when the window is resized.
    pub fn set_regions(&self, regions: &[Region]) {
        self.send(Cmd::Regions(regions.to_vec()));
    }

    /// Takes the blur off the window. It also goes by itself when the window is destroyed.
    pub fn detach(&self) {
        self.send(Cmd::Detach);
    }

    fn send(&self, cmd: Cmd) {
        if let Cmd::Style(s, _) = &cmd {
            *self.shared.style.lock().unwrap() = *s;
        }
        self.shared.queue.lock().unwrap().push(cmd);
        let hwnd = HWND(self.hwnd as *mut c_void);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return;
            }
            if on_window_thread(hwnd) {
                // Already where the engine lives: no need to wait for the message loop.
                if let Some(slot) = slot_of(hwnd) {
                    run_queue(hwnd, slot);
                }
            } else {
                let _ = PostMessageW(Some(hwnd), WM_HAZE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn on_window_thread(hwnd: HWND) -> bool {
    unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() }
}

unsafe fn slot_of(hwnd: HWND) -> Option<*mut Slot> {
    let p = GetPropW(hwnd, PROP).0 as *mut Slot;
    (!p.is_null()).then_some(p)
}

/// Carries out what was asked, on the window's thread.
unsafe fn run_queue(hwnd: HWND, slot: *mut Slot) {
    let Some(engine) = (*slot).engine.as_mut() else { return };
    let cmds = std::mem::take(&mut *engine.shared.queue.lock().unwrap());
    for cmd in cmds {
        let _ = match cmd {
            Cmd::Style(s, d) => engine.set_style(s, d),
            Cmd::Fade(v, d) => engine.fade(v, d),
            Cmd::Regions(r) => engine.set_regions(r),
            Cmd::Detach => {
                release(hwnd, slot);
                return;
            }
        };
    }
}

/// Drops the engine, and puts the window's procedure back if nobody has replaced ours since.
unsafe fn release(hwnd: HWND, slot: *mut Slot) {
    (*slot).engine = None;
    let ours = subclass_proc as *const () as usize as isize;
    if GetWindowLongPtrW(hwnd, GWLP_WNDPROC) == ours {
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, (*slot).prev);
        let _ = RemovePropW(hwnd, PROP);
        drop(Box::from_raw(slot));
    }
}

unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    const SIZE_MINIMIZED: usize = 1;
    let Some(slot) = slot_of(hwnd) else {
        return windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wp, lp);
    };
    let prev: windows::Win32::UI::WindowsAndMessaging::WNDPROC = std::mem::transmute((*slot).prev);

    match msg {
        WM_SIZE => {
            if let Some(e) = (*slot).engine.as_mut() {
                let minimized = wp.0 == SIZE_MINIMIZED;
                let _ = e.set_visible(!minimized);
                if !minimized {
                    let _ = e.refresh_size();
                }
            }
        }
        WM_SHOWWINDOW => {
            if let Some(e) = (*slot).engine.as_ref() {
                let _ = e.set_visible(wp.0 != 0);
            }
        }
        WM_DPICHANGED => {
            if let Some(e) = (*slot).engine.as_mut() {
                let _ = e.refresh_size();
            }
        }
        WM_HAZE => {
            run_queue(hwnd, slot);
            return LRESULT(0);
        }
        WM_DESTROY => {
            // The compositor has to be let go while the window is still whole: tearing it down in
            // WM_NCDESTROY, when the window is already half gone, crashed the process.
            (*slot).engine = None;
        }
        WM_NCDESTROY => {
            // The window is gone: the slot goes with it, after the old procedure has run.
            let _ = RemovePropW(hwnd, PROP);
            (*slot).engine = None;
            let r = CallWindowProcW(prev, hwnd, msg, wp, lp);
            drop(Box::from_raw(slot));
            return r;
        }
        _ => {}
    }
    CallWindowProcW(prev, hwnd, msg, wp, lp)
}
