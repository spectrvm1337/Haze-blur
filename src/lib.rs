//! **Haze**: a rounded-corner backdrop blur for Windows 10 windows.
//!
//! Windows 10 can only blur a whole rectangle behind a window (the acrylic and blur accents), so a
//! transparent window with a rounded card in it gets a blur with square corners sticking out. Haze
//! draws the blur itself with `Windows.UI.Composition`: one sprite painted with the `HostBackdropBrush`
//! (what is behind the window), a Gaussian blur on top of the system's own, and a rounded-rectangle
//! clip that gives the blur its shape. It sits under the window's content, so a transparent webview
//! or a layered child window draws over it.
//!
//! ```no_run
//! # fn demo(hwnd: *mut std::ffi::c_void) -> windows::core::Result<()> {
//! use std::time::Duration;
//! use haze::{Haze, Style};
//!
//! // On the thread that owns the window.
//! let haze = Haze::new(hwnd, Style { radius: 20.0, strength: 10.0 })?;
//! haze.resize(480.0, 700.0, 1.25);               // client size in pixels, and the DPI scale
//! haze.fade(false, Duration::from_millis(200));  // fade the blur out with your own fade
//! # Ok(()) }
//! ```
//!
//! Dropping the [`Haze`] removes the blur.
//!
//! It uses `SetWindowCompositionAttribute`, which is undocumented, to let the brush see behind the
//! window (accent state 5, `ACCENT_ENABLE_HOSTBACKDROP`).
#![cfg(windows)]

use std::cell::Cell;
use std::ffi::c_void;
use std::time::Duration;

use windows::core::{implement, Interface, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionBrush, CompositionEffectSourceParameter, Compositor, ContainerVisual};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};

pub use windows::core::{Error, Result};

/// How the blur looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Corner radius in device-independent pixels (CSS px). [`Haze::resize`] scales it.
    pub radius: f32,
    /// Extra Gaussian blur on top of the system's own, in px.
    pub strength: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style { radius: 20.0, strength: 10.0 }
    }
}

/// A rounded blur under one window. Not `Send`: it belongs to the thread that created it.
pub struct Haze {
    // Fields drop in the order they are declared: the visuals first, the compositor after them, and
    // the dispatcher queue they all run on last of all.
    root: ContainerVisual,
    _target: DesktopWindowTarget,
    compositor: Compositor,
    brush: CompositionBrush,
    _queue: Option<windows::System::DispatcherQueueController>,
    hwnd: HWND,
    style: Style,
    size: Cell<(f32, f32, f32)>,
}

impl Haze {
    /// Starts the blur on `hwnd`, a raw `HWND` (`window.hwnd()` in Tauri, `RawWindowHandle` in winit).
    /// The window has to be transparent, or have no background of its own, for the blur to show.
    /// Call it on the thread that owns the window. Nothing is drawn until the first [`resize`](Self::resize).
    pub fn new(hwnd: *mut c_void, style: Style) -> Result<Haze> {
        use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
        use windows::Win32::System::WinRT::{
            CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
        };

        let hwnd = HWND(hwnd);
        // Lets the brush see what is behind the window.
        unsafe { set_accent(hwnd, ACCENT_ENABLE_HOSTBACKDROP) };

        // A compositor needs a dispatcher queue on its thread; if the thread already has one, that one does.
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })
        }
        .ok();

        let built = (|| {
            let compositor = Compositor::new()?;
            let interop: ICompositorDesktopInterop = compositor.cast()?;
            // Not topmost: the sprite goes under the window's content, which is drawn over it.
            let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
            let root = compositor.CreateContainerVisual()?;
            target.SetRoot(&root)?;
            let brush = make_brush(&compositor, style.strength)?;
            Ok(Haze { root, _target: target, compositor, brush, _queue: queue, hwnd, style, size: Cell::new((0.0, 0.0, 1.0)) })
        })();
        if built.is_err() {
            unsafe { set_accent(hwnd, ACCENT_DISABLED) };
        }
        built
    }

    /// The current style.
    pub fn style(&self) -> Style {
        self.style
    }

    /// Changes the corner radius and the strength, and redraws.
    pub fn set_style(&mut self, style: Style) -> Result<()> {
        if style.strength != self.style.strength {
            self.brush = make_brush(&self.compositor, style.strength)?;
        }
        self.style = style;
        let (w, h, scale) = self.size.get();
        if w > 0.0 && h > 0.0 {
            self.resize(w, h, scale)?;
        }
        Ok(())
    }

    /// Redraws the blur to the window's client size in pixels. `scale` is the DPI scale (1.25 at
    /// 120 DPI); it turns [`Style::radius`] into pixels. Call it whenever the window is resized or
    /// moved to a monitor with another scale.
    pub fn resize(&self, width: f32, height: f32, scale: f32) -> Result<()> {
        use windows_numerics::Vector2;
        self.size.set((width, height, scale));
        let radius = self.style.radius * scale;

        let kids = self.root.Children()?;
        kids.RemoveAll()?;
        let sprite = self.compositor.CreateSpriteVisual()?;
        sprite.SetBrush(&self.brush)?;
        sprite.SetSize(Vector2 { X: width, Y: height })?;
        let shape = self.compositor.CreateRoundedRectangleGeometry()?;
        shape.SetSize(Vector2 { X: width, Y: height })?;
        shape.SetCornerRadius(Vector2 { X: radius, Y: radius })?;
        sprite.SetClip(&self.compositor.CreateGeometricClipWithGeometry(&shape)?)?;
        kids.InsertAtTop(&sprite)
    }

    /// Fades the blur in or out over `duration`, or at once if it is zero. A page that fades its own
    /// content leaves the blur at full strength unless it is faded with it. The curve is
    /// `cubic-bezier(0.32, 0.72, 0, 1)`; [`fade_with`](Self::fade_with) takes another.
    pub fn fade(&self, visible: bool, duration: Duration) -> Result<()> {
        self.fade_with(visible, duration, [0.32, 0.72, 0.0, 1.0])
    }

    /// Like [`fade`](Self::fade), on a CSS-style cubic bezier `[x1, y1, x2, y2]`.
    pub fn fade_with(&self, visible: bool, duration: Duration, bezier: [f32; 4]) -> Result<()> {
        use windows_numerics::Vector2;
        let to = if visible { 1.0 } else { 0.0 };
        let name = HSTRING::from("Opacity");
        self.root.StopAnimation(&name)?;
        if duration.is_zero() {
            return self.root.SetOpacity(to);
        }
        let c = &self.compositor;
        let [x1, y1, x2, y2] = bezier;
        let anim = c.CreateScalarKeyFrameAnimation()?;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: x1, Y: y1 }, Vector2 { X: x2, Y: y2 })?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
        let ticks = duration.min(Duration::from_secs(5)).as_nanos() / 100; // 100 ns units
        anim.SetDuration(windows::Foundation::TimeSpan { Duration: ticks as i64 })?;
        self.root.StartAnimation(&name, &anim)
    }
}

impl Drop for Haze {
    fn drop(&mut self) {
        unsafe { set_accent(self.hwnd, ACCENT_DISABLED) };
    }
}

// ---- accent -----------------------------------------------------------------------------------

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_HOSTBACKDROP: u32 = 5;
const WCA_ACCENT_POLICY: u32 = 19;

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionAttribData {
    attrib: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttribData) -> i32;

/// `SetWindowCompositionAttribute` is undocumented, so it is looked up rather than linked.
unsafe fn set_accent(hwnd: HWND, state: u32) {
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    let Ok(user32) = GetModuleHandleA(s!("user32.dll")) else { return };
    let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
    let f: SetWindowCompositionAttribute = std::mem::transmute(f);
    let mut policy = AccentPolicy { state, flags: 2, gradient: 0x0100_0000, animation: 0 };
    let mut data = CompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        data: &mut policy as *mut _ as *mut c_void,
        size: std::mem::size_of::<AccentPolicy>(),
    };
    f(hwnd, &mut data);
}

// ---- the brush --------------------------------------------------------------------------------

/// The system's blurred backdrop, blurred `strength` px more.
fn make_brush(c: &Compositor, strength: f32) -> Result<CompositionBrush> {
    let backdrop: CompositionBrush = c.CreateHostBackdropBrush()?.cast()?;
    let source: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&HSTRING::from("backdrop"))?.cast()?;
    let effect: IGraphicsEffect = GaussianBlur { amount: strength, source }.into();
    let brush = c.CreateEffectFactory(&effect)?.CreateBrush()?;
    brush.SetSourceParameter(&HSTRING::from("backdrop"), &backdrop)?;
    brush.cast()
}

// ---- the Gaussian blur effect -----------------------------------------------------------------
// Composition takes effects as IGraphicsEffect objects that describe a Direct2D effect (what Win2D
// provides in C#, and Win2D does not run in desktop apps). Only the one effect is needed, so it is
// described here.

/// CLSID_D2D1GaussianBlur.
const D2D1_GAUSSIAN_BLUR: windows::core::GUID = windows::core::GUID::from_u128(0x1feb6d69_2fe6_4ac9_8c58_1d7f93e7a6a5);
/// D2D1_BORDER_MODE_HARD: the edges are not blurred into transparency.
const BORDER_MODE_HARD: u32 = 1;
/// D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.
const OPTIMIZATION_BALANCED: u32 = 1;

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlur {
    amount: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffect_Impl for GaussianBlur_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(HSTRING::from("blur"))
    }
    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for GaussianBlur_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlur_Impl {
    fn GetEffectId(&self) -> Result<windows::core::GUID> {
        Ok(D2D1_GAUSSIAN_BLUR)
    }
    // No property is animated, so none is looked up by name.
    fn GetNamedPropertyMapping(&self, _: &windows::core::PCWSTR, _: *mut u32, _: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING) -> Result<()> {
        Err(windows::Win32::Foundation::E_INVALIDARG.into())
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }
    // In D2D1_GAUSSIANBLUR_PROP order: standard deviation, optimization, border mode.
    fn GetProperty(&self, index: u32) -> Result<windows::Foundation::IPropertyValue> {
        use windows::Foundation::PropertyValue;
        match index {
            0 => PropertyValue::CreateSingle(self.amount)?.cast(),
            1 => PropertyValue::CreateUInt32(OPTIMIZATION_BALANCED)?.cast(),
            2 => PropertyValue::CreateUInt32(BORDER_MODE_HARD)?.cast(),
            _ => Err(windows::Win32::Foundation::E_INVALIDARG.into()),
        }
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}
