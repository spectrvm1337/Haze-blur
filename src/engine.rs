//! The Composition side: lives on the window's thread, owned by the window's subclass.
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

use windows::core::{Interface, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffectSource};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionAnimation, CompositionColorBrush, CompositionEffectBrush, CompositionEffectSourceParameter,
    CompositionRoundedRectangleGeometry, Compositor, ContainerVisual, SpriteVisual,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_numerics::Vector2;

use crate::effect::{GaussianBlur, AMOUNT, EFFECT_NAME};
use crate::{Shared, Style};

/// The page's usual easing curve; the fade and the animated style changes run on it.
const BEZIER: [f32; 4] = [0.32, 0.72, 0.0, 1.0];

pub(crate) struct Engine {
    // Fields drop in the order they are declared: the visuals first, the compositor after them, and
    // the dispatcher queue they all run on last of all.
    root: ContainerVisual,
    blur_layer: SpriteVisual,
    tint_layer: SpriteVisual,
    blur_shape: CompositionRoundedRectangleGeometry,
    tint_shape: CompositionRoundedRectangleGeometry,
    brush: CompositionEffectBrush,
    tint_brush: CompositionColorBrush,
    _target: DesktopWindowTarget,
    compositor: Compositor,
    _queue: Option<windows::System::DispatcherQueueController>,
    hwnd: HWND,
    pub(crate) shared: Arc<Shared>,
    style: Style,
    size: (f32, f32),
    scale: f32,
}

impl Engine {
    pub(crate) fn new(hwnd: HWND, style: Style, shared: Arc<Shared>) -> windows::core::Result<Engine> {
        use windows_collections::IIterable;
        use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
        use windows::Win32::System::WinRT::{
            CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
        };

        // Lets the brush see what is behind the window.
        unsafe { set_accent(hwnd, ACCENT_ENABLE_HOSTBACKDROP) };
        // From here on, a failure has to put the accent back.
        let undo = Undo(hwnd);

        // A compositor needs a dispatcher queue on its thread; if the thread already has one, that one does.
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })
        }
        .ok();

        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        // Not topmost: the layers go under the window's content, which is drawn over them.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;

        // The blur: the system's blurred backdrop, blurred some more by an amount that can be animated.
        let backdrop: windows::UI::Composition::CompositionBrush = compositor.CreateHostBackdropBrush()?.cast()?;
        let source: IGraphicsEffectSource =
            CompositionEffectSourceParameter::Create(&HSTRING::from("backdrop"))?.cast()?;
        let effect: IGraphicsEffect = GaussianBlur { amount: style.strength, source }.into();
        let animated: IIterable<HSTRING> = vec![HSTRING::from(prop_name())].into();
        let brush = compositor.CreateEffectFactoryWithProperties(&effect, &animated)?.CreateBrush()?;
        brush.SetSourceParameter(&HSTRING::from("backdrop"), &backdrop)?;
        brush.Properties()?.InsertScalar(&HSTRING::from(prop_name()), style.strength)?;

        let (blur_layer, blur_shape) = layer(&compositor, &brush.cast::<windows::UI::Composition::CompositionBrush>()?)?;

        // The dark layer over it: always black-ish with an opacity, so that the opacity can be animated.
        let tint_brush = compositor.CreateColorBrushWithColor(color_of(style.tint_color))?;
        let (tint_layer, tint_shape) = layer(&compositor, &tint_brush.cast::<windows::UI::Composition::CompositionBrush>()?)?;
        tint_layer.SetOpacity(style.tint.clamp(0.0, 1.0))?;

        let kids = root.Children()?;
        kids.InsertAtTop(&blur_layer)?;
        kids.InsertAtTop(&tint_layer)?;

        std::mem::forget(undo);
        let mut engine = Engine {
            root,
            blur_layer,
            tint_layer,
            blur_shape,
            tint_shape,
            brush,
            tint_brush,
            _target: target,
            compositor,
            _queue: queue,
            hwnd,
            shared,
            style,
            size: (0.0, 0.0),
            scale: 1.0,
        };
        engine.refresh_size()?;
        Ok(engine)
    }

    /// Reads the window's client size and DPI scale, and fits the layers to them.
    pub(crate) fn refresh_size(&mut self) -> windows::core::Result<()> {
        let mut r = Default::default();
        unsafe { GetClientRect(self.hwnd, &mut r)? };
        let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        self.scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        if w <= 0.0 || h <= 0.0 {
            return Ok(()); // minimized
        }
        self.size = (w, h);
        let size = Vector2 { X: w, Y: h };
        self.root.SetSize(size)?;
        for (layer, shape) in [(&self.blur_layer, &self.blur_shape), (&self.tint_layer, &self.tint_shape)] {
            layer.SetSize(size)?;
            shape.SetSize(size)?;
        }
        self.set_radius_now()
    }

    fn set_radius_now(&self) -> windows::core::Result<()> {
        let r = self.style.radius * self.scale;
        for shape in [&self.blur_shape, &self.tint_shape] {
            shape.SetCornerRadius(Vector2 { X: r, Y: r })?;
        }
        Ok(())
    }

    /// Moves to `style` over `duration` (at once if it is zero). The animations run in the compositor,
    /// not in the app, so nothing is called per frame.
    pub(crate) fn set_style(&mut self, style: Style, duration: Duration) -> windows::core::Result<()> {
        use windows_numerics::Vector2;
        self.style = style;
        let strength_prop = HSTRING::from(prop_name());
        let radius = style.radius * self.scale;
        let tint = style.tint.clamp(0.0, 1.0);

        // Whatever was running is replaced by the new target.
        for shape in [&self.blur_shape, &self.tint_shape] {
            shape.StopAnimation(&HSTRING::from("CornerRadius"))?;
        }
        self.brush.StopAnimation(&strength_prop)?;
        self.tint_layer.StopAnimation(&HSTRING::from("Opacity"))?;
        self.tint_brush.StopAnimation(&HSTRING::from("Color"))?;

        if duration.is_zero() {
            self.set_radius_now()?;
            self.brush.Properties()?.InsertScalar(&strength_prop, style.strength)?;
            self.tint_layer.SetOpacity(tint)?;
            return self.tint_brush.SetColor(color_of(style.tint_color));
        }

        let c = &self.compositor;
        let dur = timespan(duration);
        let ease = easing(c)?;

        let a = c.CreateVector2KeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, Vector2 { X: radius, Y: radius }, &ease)?;
        a.SetDuration(dur)?;
        for shape in [&self.blur_shape, &self.tint_shape] {
            shape.StartAnimation(&HSTRING::from("CornerRadius"), &a.cast::<CompositionAnimation>()?)?;
        }

        let a = c.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, style.strength, &ease)?;
        a.SetDuration(dur)?;
        self.brush.StartAnimation(&strength_prop, &a)?;

        let a = c.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, tint, &ease)?;
        a.SetDuration(dur)?;
        self.tint_layer.StartAnimation(&HSTRING::from("Opacity"), &a)?;

        let a = c.CreateColorKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, color_of(style.tint_color), &ease)?;
        a.SetDuration(dur)?;
        self.tint_brush.StartAnimation(&HSTRING::from("Color"), &a)
    }

    /// Fades the whole blur in or out.
    pub(crate) fn fade(&self, visible: bool, duration: Duration) -> windows::core::Result<()> {
        let to = if visible { 1.0 } else { 0.0 };
        let name = HSTRING::from("Opacity");
        self.root.StopAnimation(&name)?;
        if duration.is_zero() {
            return self.root.SetOpacity(to);
        }
        let a = self.compositor.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, to, &easing(&self.compositor)?)?;
        a.SetDuration(timespan(duration))?;
        self.root.StartAnimation(&name, &a)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { set_accent(self.hwnd, ACCENT_DISABLED) };
    }
}

/// Puts the accent back if building the engine fails halfway.
struct Undo(HWND);
impl Drop for Undo {
    fn drop(&mut self) {
        unsafe { set_accent(self.0, ACCENT_DISABLED) };
    }
}

fn prop_name() -> String {
    format!("{EFFECT_NAME}.{AMOUNT}")
}

fn color_of(rgb: [u8; 3]) -> windows::UI::Color {
    windows::UI::Color { A: 255, R: rgb[0], G: rgb[1], B: rgb[2] }
}

fn timespan(d: Duration) -> windows::Foundation::TimeSpan {
    // 100 ns units; a few seconds is plenty
    windows::Foundation::TimeSpan { Duration: (d.min(Duration::from_secs(10)).as_nanos() / 100) as i64 }
}

fn easing(c: &Compositor) -> windows::core::Result<windows::UI::Composition::CubicBezierEasingFunction> {
    let [x1, y1, x2, y2] = BEZIER;
    c.CreateCubicBezierEasingFunction(Vector2 { X: x1, Y: y1 }, Vector2 { X: x2, Y: y2 })
}

/// One layer: a sprite painted with `brush`, cut to a rounded rectangle whose size is set later.
fn layer(
    c: &Compositor,
    brush: &windows::UI::Composition::CompositionBrush,
) -> windows::core::Result<(SpriteVisual, CompositionRoundedRectangleGeometry)> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetBrush(brush)?;
    let shape = c.CreateRoundedRectangleGeometry()?;
    sprite.SetClip(&c.CreateGeometricClipWithGeometry(&shape)?)?;
    Ok((sprite, shape))
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
