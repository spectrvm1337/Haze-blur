//! **Haze**: a rounded-corner backdrop blur for Windows 10 windows.
//!
//! Windows 10 can only blur a whole rectangle behind a window (the acrylic and blur accents), so a
//! transparent window with a rounded card in it gets a blur with square corners sticking out. Haze
//! draws the blur itself with `Windows.UI.Composition`: a sprite painted with the `HostBackdropBrush`
//! (what is behind the window), run through a Gaussian blur and a tint, and clipped to a rounded
//! rectangle or a squircle. It sits under the window's content, so a transparent webview or a layered
//! child window draws over it.
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
mod shape;
mod window;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, PostMessageW};

pub use windows::core::{Error, Result};

/// How the blur looks. Build one from a preset and change what you need:
/// `Style::dark().radius(24.0).strength(18.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Style {
    /// Corner radius in device-independent pixels (CSS px); it is scaled by the window's DPI.
    pub radius: f32,
    /// Extra Gaussian blur on top of the system's own, in CSS px; it is scaled by the window's DPI.
    pub strength: f32,
    /// How much of the tint colour is laid over the blur, 0.0 (none) to 1.0 (solid). A blur over a
    /// light or busy backdrop is hard to see; tinting it makes the shape stand out.
    pub tint: f32,
    /// The tint's colour, RGB.
    pub tint_color: [u8; 3],
    /// Colour saturation of what shows through: 0 is grey, 1 leaves the colours alone, above 1
    /// strengthens them.
    pub saturation: f32,
    /// The shape of the corners.
    pub corner: Corner,
}

impl Default for Style {
    /// Plain blur, no tint.
    fn default() -> Self {
        Style { radius: 20.0, strength: 10.0, tint: 0.0, tint_color: [0, 0, 0], saturation: 1.0, corner: Corner::Round }
    }
}

impl Style {
    /// Dark glass: a graphite tint, easy to read light text on.
    pub fn dark() -> Style {
        Style { strength: 14.0, tint: 0.45, tint_color: [10, 10, 14], ..Style::default() }
    }
    /// Light glass.
    pub fn light() -> Style {
        Style { strength: 14.0, tint: 0.4, tint_color: [244, 245, 250], ..Style::default() }
    }
    /// Frosted: a strong blur with only a trace of white.
    pub fn frost() -> Style {
        Style { strength: 24.0, tint: 0.16, tint_color: [255, 255, 255], saturation: 1.25, ..Style::default() }
    }
    /// Corner radius in CSS px.
    pub fn radius(mut self, radius: f32) -> Style {
        self.radius = radius;
        self.sanitized()
    }
    /// Blur strength in CSS px.
    pub fn strength(mut self, strength: f32) -> Style {
        self.strength = strength;
        self.sanitized()
    }
    /// Tint opacity, 0.0 to 1.0.
    pub fn tint(mut self, tint: f32) -> Style {
        self.tint = tint;
        self.sanitized()
    }
    /// Tint colour.
    pub fn tint_color(mut self, rgb: [u8; 3]) -> Style {
        self.tint_color = rgb;
        self
    }
    /// Colour saturation of what shows through; 1 leaves it alone.
    pub fn saturation(mut self, saturation: f32) -> Style {
        self.saturation = saturation;
        self.sanitized()
    }
    /// The shape of the corners.
    pub fn corner(mut self, corner: Corner) -> Style {
        self.corner = corner;
        self.sanitized()
    }

    /// The fields are public, so whatever arrives is brought into range before it is used.
    pub(crate) fn sanitized(self) -> Style {
        Style {
            radius: finite(self.radius, 0.0).max(0.0),
            strength: finite(self.strength, 0.0).max(0.0),
            tint: finite(self.tint, 0.0).clamp(0.0, 1.0),
            tint_color: self.tint_color,
            saturation: finite(self.saturation, 1.0).max(0.0),
            corner: self.corner.sanitized(),
        }
    }
}

/// The shape of a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Corner {
    /// A circular arc.
    #[default]
    Round,
    /// A squircle, CSS `corner-shape: squircle`: it leaves the straight edge more gently than an arc.
    Squircle,
    /// CSS `superellipse(k)`: 1 is round, 2 a squircle, higher is squarer, 0 a straight bevel.
    Superellipse(f32),
}

impl Corner {
    /// The superellipse exponent `k` of CSS `superellipse(k)`.
    pub(crate) fn k(self) -> f32 {
        match self {
            Corner::Round => 1.0,
            Corner::Squircle => 2.0,
            Corner::Superellipse(k) => k,
        }
    }
    pub(crate) fn is_round(self) -> bool {
        (self.k() - 1.0).abs() < 0.01
    }
    fn sanitized(self) -> Corner {
        match self {
            Corner::Superellipse(k) if !k.is_finite() => Corner::Round,
            Corner::Superellipse(k) => Corner::Superellipse(k.clamp(0.0, 8.0)),
            other => other,
        }
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
    /// This region's own corner shape, instead of the style's.
    pub corner: Option<Corner>,
    /// 0.0 to 1.0, so the blur of one card can fade with it.
    pub opacity: f32,
}

impl Region {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Region {
        Region { x, y, width, height, radius: None, corner: None, opacity: 1.0 }
    }
    /// Gives this region its own corner radius. Half of its height makes a pill.
    pub fn radius(mut self, radius: f32) -> Region {
        self.radius = Some(radius);
        self
    }
    /// Gives this region its own corner shape.
    pub fn corner(mut self, corner: Corner) -> Region {
        self.corner = Some(corner);
        self
    }
    /// The region's opacity, 0.0 to 1.0.
    pub fn opacity(mut self, opacity: f32) -> Region {
        self.opacity = opacity;
        self
    }

    /// `None` for a region that cannot be drawn: an empty one, or one with a coordinate that is not a number.
    pub(crate) fn sanitized(self) -> Option<Region> {
        let fine = [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite());
        (fine && self.width > 0.0 && self.height > 0.0).then(|| Region {
            radius: self.radius.filter(|r| r.is_finite()).map(|r| r.max(0.0)),
            corner: self.corner.map(Corner::sanitized),
            opacity: finite(self.opacity, 1.0).clamp(0.0, 1.0),
            ..self
        })
    }
}

/// How a change gets to where it is going: over how long, after how long, and on what curve.
/// A [`Duration`] converts into one, so `animate_to(style, Duration::from_millis(300))` works.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Transition {
    pub duration: Duration,
    /// How long to wait before starting, for example to keep pace with a webview, whose frames reach
    /// the screen a frame or two after the compositor's.
    pub delay: Duration,
    pub easing: Easing,
}

impl Transition {
    /// At once.
    pub const INSTANT: Transition = Transition { duration: Duration::ZERO, delay: Duration::ZERO, easing: Easing::DEFAULT };

    pub fn new(duration: Duration) -> Transition {
        Transition { duration, ..Transition::INSTANT }
    }
    pub fn delay(mut self, delay: Duration) -> Transition {
        self.delay = delay;
        self
    }
    pub fn easing(mut self, easing: Easing) -> Transition {
        self.easing = easing;
        self
    }
    pub(crate) fn is_instant(&self) -> bool {
        self.duration.is_zero() && self.delay.is_zero()
    }
}

impl From<Duration> for Transition {
    fn from(duration: Duration) -> Transition {
        Transition::new(duration)
    }
}

/// The curve a [`Transition`] runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`.
    CubicBezier([f32; 4]),
}

impl Easing {
    /// `cubic-bezier(0.32, 0.72, 0, 1)`: quick to start, long and soft to land.
    pub const DEFAULT: Easing = Easing::CubicBezier([0.32, 0.72, 0.0, 1.0]);
}

impl Default for Easing {
    fn default() -> Easing {
        Easing::DEFAULT
    }
}

/// What has been asked of the blur and not carried out yet. Only the latest of each kind matters, so
/// a burst of calls from another thread costs one round on the window's thread.
#[derive(Default)]
pub(crate) struct Pending {
    pub(crate) style: Option<(Style, Transition)>,
    pub(crate) fade: Option<(bool, Transition)>,
    /// `Some(None)`: the whole window.
    pub(crate) regions: Option<Option<Vec<Region>>>,
    pub(crate) scale: Option<Option<f32>>,
    pub(crate) detach: bool,
}

impl Pending {
    pub(crate) fn is_empty(&self) -> bool {
        self.style.is_none() && self.fade.is_none() && self.regions.is_none() && self.scale.is_none() && !self.detach
    }
    /// `self` with whatever `newer` asks for on top.
    pub(crate) fn then(self, newer: Pending) -> Pending {
        Pending {
            style: newer.style.or(self.style),
            fade: newer.fade.or(self.fade),
            regions: newer.regions.or(self.regions),
            scale: newer.scale.or(self.scale),
            detach: self.detach || newer.detach,
        }
    }
}

/// What the handles and the window share.
pub(crate) struct Shared {
    hwnd: isize,
    /// False once the blur is detached or the window is gone; calls do nothing after that.
    pub(crate) alive: AtomicBool,
    /// Whether a message is on its way to the window already.
    pub(crate) posted: AtomicBool,
    pub(crate) pending: Mutex<Pending>,
    style: Mutex<Style>,
    error: Mutex<Option<Error>>,
}

impl Shared {
    pub(crate) fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }
    /// Wakes the window's thread, unless it has been woken already.
    pub(crate) fn post(&self) {
        if self.posted.swap(true, Ordering::AcqRel) {
            return;
        }
        let sent = unsafe { PostMessageW(Some(self.hwnd()), window::wm_haze(), WPARAM(0), LPARAM(0)) };
        if sent.is_err() {
            self.posted.store(false, Ordering::Release);
        }
    }
    pub(crate) fn record(&self, result: Result<()>) {
        if let Err(e) = result {
            *lock(&self.error) = Some(e);
        }
    }
}

/// A rounded blur under one window. Cheap to clone and safe to send to other threads; the calls are
/// carried out on the window's own thread. Dropping a handle leaves the blur where it is.
#[derive(Clone)]
pub struct Haze {
    shared: Arc<Shared>,
}

impl Haze {
    /// Whether this Windows has what the blur needs (Windows 10 1809 or newer).
    pub fn supported() -> bool {
        use windows::core::HSTRING;
        use windows::Foundation::Metadata::ApiInformation;
        let backdrop =
            ApiInformation::IsMethodPresent(&HSTRING::from("Windows.UI.Composition.Compositor"), &HSTRING::from("CreateHostBackdropBrush"));
        let clip = ApiInformation::IsTypePresent(&HSTRING::from("Windows.UI.Composition.CompositionGeometricClip"));
        backdrop.unwrap_or(false) && clip.unwrap_or(false)
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
        use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_INVALID_WINDOW_HANDLE, RPC_E_WRONG_THREAD};
        let hwnd = HWND(hwnd);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(ERROR_INVALID_WINDOW_HANDLE.to_hresult().into());
        }
        if !on_window_thread(hwnd) {
            return Err(Error::new(RPC_E_WRONG_THREAD, "attach has to run on the window's thread"));
        }
        if window::find(hwnd).is_some() {
            return Err(ERROR_ALREADY_EXISTS.to_hresult().into());
        }
        let style = style.sanitized();
        let engine = engine::Engine::new(hwnd, style)?;
        let shared = Arc::new(Shared {
            hwnd: hwnd.0 as isize,
            alive: AtomicBool::new(true),
            posted: AtomicBool::new(false),
            pending: Mutex::new(Pending::default()),
            style: Mutex::new(style),
            error: Mutex::new(None),
        });
        window::install(hwnd, shared.clone(), engine)?;
        Ok(Haze { shared })
    }

    /// The style that was asked for last.
    pub fn style(&self) -> Style {
        *lock(&self.shared.style)
    }

    /// Moves to `style` at once.
    pub fn set_style(&self, style: Style) {
        self.animate_to(style, Transition::INSTANT);
    }

    /// Moves to `style` over a [`Transition`] (or a plain `Duration`). The radius, the strength, the tint,
    /// its colour and the saturation all move together; the compositor runs the animation.
    pub fn animate_to(&self, style: Style, transition: impl Into<Transition>) {
        let style = style.sanitized();
        *lock(&self.shared.style) = style;
        let transition = transition.into();
        self.send(|p| p.style = Some((style, transition)));
    }

    /// Fades the blur in or out. A page that fades its own content leaves the blur at full strength
    /// unless it is faded with it.
    pub fn fade(&self, visible: bool, transition: impl Into<Transition>) {
        let transition = transition.into();
        self.send(|p| p.fade = Some((visible, transition)));
    }

    /// Blurs only these parts of the window, for example a sidebar and a card. An empty list blurs
    /// nothing; [`cover_window`](Self::cover_window) goes back to the whole window. The regions stay
    /// where they are when the window is resized. Calling this every frame is fine: unchanged regions
    /// cost nothing and changed ones are updated in place.
    pub fn set_regions(&self, regions: &[Region]) {
        let regions = regions.to_vec();
        self.send(|p| p.regions = Some(Some(regions)));
    }

    /// Blurs the whole window, as right after [`attach`](Self::attach).
    pub fn cover_window(&self) {
        self.send(|p| p.regions = Some(None));
    }

    /// How many physical pixels one unit of the radius, the strength and the regions is. `None`, the
    /// default, follows the window's DPI, which suits CSS px. Pass `Some(1.0)` to work in physical pixels,
    /// or a webview's `devicePixelRatio` to keep up with its zoom.
    pub fn set_scale(&self, scale: Option<f32>) {
        let scale = scale.filter(|s| s.is_finite() && *s > 0.0);
        self.send(|p| p.scale = Some(scale));
    }

    /// Takes the blur off the window. It also goes by itself when the window is destroyed.
    pub fn detach(&self) {
        self.send(|p| p.detach = true);
    }

    /// Whether the blur is still on the window: false after [`detach`](Self::detach) or once the
    /// window is destroyed. Calls on a handle that is no longer attached do nothing.
    pub fn is_attached(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }

    /// The last error the compositor reported while carrying out a call, if any. The calls themselves
    /// return nothing, since most of them run later on the window's thread.
    pub fn last_error(&self) -> Option<Error> {
        lock(&self.shared.error).clone()
    }

    fn send(&self, change: impl FnOnce(&mut Pending)) {
        let shared = &self.shared;
        if !shared.alive.load(Ordering::Acquire) {
            return;
        }
        change(&mut lock(&shared.pending));
        let hwnd = shared.hwnd();
        if on_window_thread(hwnd) {
            // Already where the engine lives: no need to wait for the message loop.
            if let Some(slot) = window::find(hwnd).filter(|slot| Arc::ptr_eq(&slot.shared, shared)) {
                slot.run();
                return;
            }
        }
        shared.post();
    }
}

fn on_window_thread(hwnd: HWND) -> bool {
    unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() }
}

/// A lock that a panic elsewhere does not poison for good.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}
