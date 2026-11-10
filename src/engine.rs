//! The Composition side: lives on the window's thread, owned by the window's subclass.
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

use windows::core::{Interface, HSTRING};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffectSource};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionColorBrush, CompositionEffectBrush, CompositionEffectSourceParameter,
    CompositionRoundedRectangleGeometry, Compositor, ContainerVisual, SpriteVisual,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_numerics::{Vector2, Vector3};

use crate::effect::{gaussian_blur, saturation, BLUR_AMOUNT, SATURATION};
use crate::{Region, Shared, Style};

/// The easing curve the fade and the animated style changes run on.
const BEZIER: [f32; 4] = [0.32, 0.72, 0.0, 1.0];

/// One blurred shape: the blur, and the tint lying over it.
struct Layer {
    blur: SpriteVisual,
    tint: SpriteVisual,
    blur_shape: CompositionRoundedRectangleGeometry,
    tint_shape: CompositionRoundedRectangleGeometry,
    /// `None` fills the window.
    region: Option<Region>,
}

impl Layer {
    fn shapes(&self) -> [&CompositionRoundedRectangleGeometry; 2] {
        [&self.blur_shape, &self.tint_shape]
    }
}

pub(crate) struct Engine {
    // Fields drop in the order they are declared: the visuals first, the compositor after them, and
    // the dispatcher queue they all run on last of all.
    layers: Vec<Layer>,
    root: ContainerVisual,
    brush: CompositionEffectBrush,
    tint_brush: CompositionColorBrush,
    _target: DesktopWindowTarget,
    compositor: Compositor,
    _queue: Option<windows::System::DispatcherQueueController>,
    hwnd: HWND,
    pub(crate) shared: Arc<Shared>,
    style: Style,
    regions: Vec<Region>,
    size: (f32, f32),
    scale: f32,
}

impl Engine {
    pub(crate) fn new(hwnd: HWND, style: Style, shared: Arc<Shared>) -> windows::core::Result<Engine> {
        use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
        use windows::Win32::System::WinRT::{
            CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
        };
        use windows_collections::IIterable;

        // Lets the brush see what is behind the window.
        unsafe {
            set_accent(hwnd, ACCENT_ENABLE_HOSTBACKDROP);
            set_host_backdrop(hwnd, true);
        }
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
        let backdrop: CompositionBrush = compositor.CreateHostBackdropBrush()?.cast()?;
        let source: IGraphicsEffectSource =
            CompositionEffectSourceParameter::Create(&HSTRING::from("backdrop"))?.cast()?;
        let saturated: IGraphicsEffectSource = saturation(style.saturation, source).cast()?;
        let effect: IGraphicsEffect = gaussian_blur(style.strength, saturated);
        let animated: IIterable<HSTRING> = vec![HSTRING::from(BLUR_AMOUNT), HSTRING::from(SATURATION)].into();
        let brush = compositor.CreateEffectFactoryWithProperties(&effect, &animated)?.CreateBrush()?;
        brush.SetSourceParameter(&HSTRING::from("backdrop"), &backdrop)?;
        let properties = brush.Properties()?;
        properties.InsertScalar(&HSTRING::from(BLUR_AMOUNT), style.strength)?;
        properties.InsertScalar(&HSTRING::from(SATURATION), style.saturation)?;

        let tint_brush = compositor.CreateColorBrushWithColor(color_of(style.tint_color))?;

        std::mem::forget(undo);
        let mut engine = Engine {
            layers: Vec::new(),
            root,
            brush,
            tint_brush,
            _target: target,
            compositor,
            _queue: queue,
            hwnd,
            shared,
            style,
            regions: Vec::new(),
            size: (0.0, 0.0),
            scale: 1.0,
        };
        engine.read_size()?;
        engine.rebuild()?;
        Ok(engine)
    }

    fn make_layer(&self, region: Option<Region>) -> windows::core::Result<Layer> {
        let (blur, blur_shape) = sprite(&self.compositor, &self.brush.cast()?)?;
        let (tint, tint_shape) = sprite(&self.compositor, &self.tint_brush.cast()?)?;
        tint.SetOpacity(self.style.tint.clamp(0.0, 1.0))?;
        Ok(Layer { blur, tint, blur_shape, tint_shape, region })
    }

    /// Makes one layer per region, or a single one that fills the window if there are no regions.
    fn rebuild(&mut self) -> windows::core::Result<()> {
        let kids = self.root.Children()?;
        kids.RemoveAll()?;
        let wanted: Vec<Option<Region>> =
            if self.regions.is_empty() { vec![None] } else { self.regions.iter().copied().map(Some).collect() };
        let mut layers = Vec::with_capacity(wanted.len());
        for region in wanted {
            let layer = self.make_layer(region)?;
            kids.InsertAtTop(&layer.blur)?;
            kids.InsertAtTop(&layer.tint)?;
            layers.push(layer);
        }
        self.layers = layers;
        self.place()
    }

    pub(crate) fn set_regions(&mut self, regions: Vec<Region>) -> windows::core::Result<()> {
        self.regions = regions;
        self.rebuild()
    }

    fn read_size(&mut self) -> windows::core::Result<bool> {
        let mut r = Default::default();
        unsafe { GetClientRect(self.hwnd, &mut r)? };
        let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        self.scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        if w <= 0.0 || h <= 0.0 {
            return Ok(false); // minimized
        }
        self.size = (w, h);
        Ok(true)
    }

    /// Reads the window's client size and DPI scale, and fits the layers to them.
    pub(crate) fn refresh_size(&mut self) -> windows::core::Result<()> {
        if self.read_size()? {
            self.place()?;
        }
        Ok(())
    }

    /// Puts every layer where it belongs for the current size, scale and style.
    fn place(&self) -> windows::core::Result<()> {
        let (cw, ch) = self.size;
        self.root.SetSize(Vector2 { X: cw, Y: ch })?;
        for l in &self.layers {
            let (x, y, w, h) = match l.region {
                Some(r) => (r.x * self.scale, r.y * self.scale, r.width * self.scale, r.height * self.scale),
                None => (0.0, 0.0, cw, ch),
            };
            let (offset, size) = (Vector3 { X: x, Y: y, Z: 0.0 }, Vector2 { X: w, Y: h });
            for visual in [&l.blur, &l.tint] {
                visual.SetOffset(offset)?;
                visual.SetSize(size)?;
            }
            let r = self.radius_of(l);
            for shape in l.shapes() {
                shape.SetSize(size)?;
                shape.SetCornerRadius(Vector2 { X: r, Y: r })?;
            }
        }
        Ok(())
    }

    /// A layer's corner radius in pixels: its region's own, or the style's.
    fn radius_of(&self, l: &Layer) -> f32 {
        l.region.and_then(|r| r.radius).unwrap_or(self.style.radius) * self.scale
    }

    /// Moves to `style` over `duration` (at once if it is zero). The animations run in the compositor,
    /// not in the app, so nothing is called per frame.
    pub(crate) fn set_style(&mut self, style: Style, duration: Duration) -> windows::core::Result<()> {
        self.style = style;
        let strength_prop = HSTRING::from(BLUR_AMOUNT);
        let saturation_prop = HSTRING::from(SATURATION);
        let (corner, opacity, color) = (HSTRING::from("CornerRadius"), HSTRING::from("Opacity"), HSTRING::from("Color"));
        let tint = style.tint.clamp(0.0, 1.0);

        // Whatever was running is replaced by the new target.
        for l in &self.layers {
            for shape in l.shapes() {
                shape.StopAnimation(&corner)?;
            }
            l.tint.StopAnimation(&opacity)?;
        }
        self.brush.StopAnimation(&strength_prop)?;
        self.brush.StopAnimation(&saturation_prop)?;
        self.tint_brush.StopAnimation(&color)?;

        if duration.is_zero() {
            let properties = self.brush.Properties()?;
            properties.InsertScalar(&strength_prop, style.strength)?;
            properties.InsertScalar(&saturation_prop, style.saturation)?;
            self.tint_brush.SetColor(color_of(style.tint_color))?;
            for l in &self.layers {
                l.tint.SetOpacity(tint)?;
            }
            return self.place();
        }

        let c = &self.compositor;
        let dur = timespan(duration);
        let ease = easing(c)?;

        for l in &self.layers {
            let r = self.radius_of(l);
            let a = c.CreateVector2KeyFrameAnimation()?;
            a.InsertKeyFrameWithEasingFunction(1.0, Vector2 { X: r, Y: r }, &ease)?;
            a.SetDuration(dur)?;
            for shape in l.shapes() {
                shape.StartAnimation(&corner, &a)?;
            }
            let a = c.CreateScalarKeyFrameAnimation()?;
            a.InsertKeyFrameWithEasingFunction(1.0, tint, &ease)?;
            a.SetDuration(dur)?;
            l.tint.StartAnimation(&opacity, &a)?;
        }

        let a = c.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, style.strength, &ease)?;
        a.SetDuration(dur)?;
        self.brush.StartAnimation(&strength_prop, &a)?;

        let a = c.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, style.saturation, &ease)?;
        a.SetDuration(dur)?;
        self.brush.StartAnimation(&saturation_prop, &a)?;

        let a = c.CreateColorKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, color_of(style.tint_color), &ease)?;
        a.SetDuration(dur)?;
        self.tint_brush.StartAnimation(&color, &a)
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

    /// Stops drawing while the window cannot be seen.
    pub(crate) fn set_visible(&self, visible: bool) -> windows::core::Result<()> {
        self.root.SetIsVisible(visible)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            set_accent(self.hwnd, ACCENT_DISABLED);
            set_host_backdrop(self.hwnd, false);
        }
    }
}

/// Puts the accent back if building the engine fails halfway.
struct Undo(HWND);
impl Drop for Undo {
    fn drop(&mut self) {
        unsafe {
            set_accent(self.0, ACCENT_DISABLED);
            set_host_backdrop(self.0, false);
        }
    }
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

/// A sprite painted with `brush`, cut to a rounded rectangle whose size is set later.
fn sprite(c: &Compositor, brush: &CompositionBrush) -> windows::core::Result<(SpriteVisual, CompositionRoundedRectangleGeometry)> {
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetBrush(brush)?;
    let shape = c.CreateRoundedRectangleGeometry()?;
    sprite.SetClip(&c.CreateGeometricClipWithGeometry(&shape)?)?;
    Ok((sprite, shape))
}

/// Windows 11's documented switch for the host backdrop brush. Windows 10 does not know the
/// attribute and refuses it, which is fine: the accent does the job there.
unsafe fn set_host_backdrop(hwnd: HWND, on: bool) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
    let value = on as i32; // a Win32 BOOL
    let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &value as *const _ as *const c_void, std::mem::size_of::<i32>() as u32);
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
