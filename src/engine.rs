//! The Composition side. Lives on the window's thread, owned by the window's subclass.
//!
//! Every amount the style animates (radius, strength, saturation, tint) lives in one property set,
//! and the brush and the clips follow it through expressions. So an animation runs on a handful of
//! values however many regions there are, and a region added or moved halfway through an animation
//! picks up where it is instead of jumping to the end.
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use windows::core::{Interface, HSTRING};
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::{D2D1CreateFactory, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsIconic, IsWindowVisible, KillTimer, SetTimer};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionColorSpace, CompositionEasingFunction, CompositionEffectFactory, CompositionGeometricClip,
    CompositionPath, CompositionPathGeometry, CompositionPropertySet, CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
    ExpressionAnimation, KeyFrameAnimation, SpriteVisual,
};
use windows_numerics::{Vector2, Vector3};

use crate::effect::{Chain, SOURCE};
use crate::{shape, Easing, Region, Style, Transition};

/// The timer that drops the effects an animation needed once it has landed. The id is unlikely to
/// collide with one of the program's own timers on the window.
pub(crate) const SETTLE_TIMER: usize = 0x4841_5A45;

/// The values in the property set, by name.
const RADIUS: &str = "Radius";
const STRENGTH: &str = "Strength";
const SATURATION: &str = "Saturation";
const TINT_COLOR: &str = "TintColor";
const SCALE: &str = "Scale";

fn hs(s: &str) -> HSTRING {
    HSTRING::from(s)
}

// ---- what a thread shares ---------------------------------------------------------------------

/// The compositor and what is built once and reused, shared by every window on one thread.
pub(crate) struct Host {
    /// Compiled effects, one per chain. Building one compiles a shader, so it is done once.
    #[allow(clippy::type_complexity)]
    factories: RefCell<[Option<(CompositionEffectFactory, Vec<(&'static str, &'static str)>)>; Chain::COUNT]>,
    d2d: RefCell<Option<ID2D1Factory>>,
    // The compositor goes before the dispatcher queue it runs on.
    compositor: Compositor,
    _queue: Option<DispatcherQueueController>,
}

thread_local! {
    static HOST: RefCell<Weak<Host>> = const { RefCell::new(Weak::new()) };
}

impl Host {
    pub(crate) fn get() -> windows::core::Result<Rc<Host>> {
        use windows::Win32::System::WinRT::{
            CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
        };
        HOST.with(|cell| {
            if let Some(host) = cell.borrow().upgrade() {
                return Ok(host);
            }
            // A compositor needs a dispatcher queue on its thread; if the thread has one already, that one does.
            let queue = unsafe {
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_NONE,
                })
            }
            .ok();
            let host = Rc::new(Host {
                factories: RefCell::new(Default::default()),
                d2d: RefCell::new(None),
                compositor: Compositor::new()?,
                _queue: queue,
            });
            *cell.borrow_mut() = Rc::downgrade(&host);
            Ok(host)
        })
    }

    #[allow(clippy::type_complexity)]
    fn factory(&self, chain: Chain) -> windows::core::Result<Option<(CompositionEffectFactory, Vec<(&'static str, &'static str)>)>> {
        if let Some(made) = &self.factories.borrow()[chain.index()] {
            return Ok(Some(made.clone()));
        }
        let Some((effect, bound)) = chain.effect()? else { return Ok(None) };
        let names: Vec<HSTRING> = bound.iter().map(|(property, _)| hs(property)).collect();
        let names: windows_collections::IIterable<HSTRING> = names.into();
        let factory = self.compositor.CreateEffectFactoryWithProperties(&effect, &names)?;
        self.factories.borrow_mut()[chain.index()] = Some((factory.clone(), bound.clone()));
        Ok(Some((factory, bound)))
    }

    fn d2d(&self) -> windows::core::Result<ID2D1Factory> {
        let mut d2d = self.d2d.borrow_mut();
        if let Some(f) = d2d.as_ref() {
            return Ok(f.clone());
        }
        let f: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        *d2d = Some(f.clone());
        Ok(f)
    }

    fn expression(&self, props: &CompositionPropertySet, text: &str) -> windows::core::Result<ExpressionAnimation> {
        let e = self.compositor.CreateExpressionAnimationWithExpression(&hs(text))?;
        e.SetReferenceParameter(&hs("s"), props)?;
        Ok(e)
    }

    /// A brush that paints the backdrop through `chain`, its amounts following `props`.
    fn brush(&self, props: &CompositionPropertySet, chain: Chain) -> windows::core::Result<CompositionBrush> {
        let backdrop: CompositionBrush = self.compositor.CreateHostBackdropBrush()?.cast()?;
        let Some((factory, bound)) = self.factory(chain)? else { return Ok(backdrop) };
        let brush = factory.CreateBrush()?;
        brush.SetSourceParameter(&hs(SOURCE), &backdrop)?;
        for (property, expression) in bound {
            brush.StartAnimation(&hs(property), &self.expression(props, expression)?)?;
        }
        brush.cast()
    }

    fn easing(&self, easing: Easing) -> windows::core::Result<CompositionEasingFunction> {
        match easing {
            Easing::Linear => self.compositor.CreateLinearEasingFunction()?.cast(),
            Easing::CubicBezier([x1, y1, x2, y2]) => self
                .compositor
                .CreateCubicBezierEasingFunction(Vector2 { X: x1.clamp(0.0, 1.0), Y: y1 }, Vector2 { X: x2.clamp(0.0, 1.0), Y: y2 })?
                .cast(),
        }
    }
}

/// The links a style needs: none of them changes anything at its neutral value.
fn chain_of(style: &Style) -> Chain {
    Chain::NONE
        .with(Chain::BLUR, style.strength > 0.0)
        .with(Chain::SATURATION, (style.saturation - 1.0).abs() > 1e-3)
        .with(Chain::TINT, style.tint > 0.0)
}

fn tint_color(style: &Style) -> windows::UI::Color {
    let [r, g, b] = style.tint_color;
    windows::UI::Color { A: (style.tint * 255.0).round() as u8, R: r, G: g, B: b }
}

fn timespan(d: Duration) -> windows::Foundation::TimeSpan {
    // 100 ns units
    windows::Foundation::TimeSpan { Duration: (d.as_nanos() / 100).min(i64::MAX as u128) as i64 }
}

/// Gives an animation the transition's timing. The compositor wants at least a millisecond.
fn time(animation: &impl Interface, t: &Transition) -> windows::core::Result<()> {
    let a: KeyFrameAnimation = animation.cast()?;
    a.SetDuration(timespan(t.duration.max(Duration::from_millis(1))))?;
    if !t.delay.is_zero() {
        a.SetDelayTime(timespan(t.delay))?;
    }
    Ok(())
}

// ---- one window -------------------------------------------------------------------------------

/// One blurred shape.
struct Layer {
    sprite: SpriteVisual,
    clip: CompositionGeometricClip,
    shape: Shape,
    /// `None` fills the window.
    region: Option<Region>,
}

enum Shape {
    /// A rounded rectangle. `follows_style`: its radius is bound to the style's.
    Round { geometry: CompositionRoundedRectangleGeometry, follows_style: bool },
    /// A path, for any other corner, with the width, height, radius and `k` it was drawn for.
    Path { geometry: CompositionPathGeometry, drawn: [f32; 4] },
}

pub(crate) struct Engine {
    // Fields drop in the order they are declared: the visuals first, the compositor last.
    layers: Vec<Layer>,
    brush: CompositionBrush,
    root: ContainerVisual,
    _target: DesktopWindowTarget,
    props: CompositionPropertySet,
    host: Rc<Host>,
    hwnd: HWND,
    /// Where the style is going.
    style: Style,
    chain: Chain,
    /// `None`: the whole window.
    regions: Option<Vec<Region>>,
    size: (f32, f32),
    dpi_scale: f32,
    scale: Option<f32>,
    /// Where the fade is going.
    shown: bool,
    accent_before: Option<AccentPolicy>,
}

impl Engine {
    pub(crate) fn new(hwnd: HWND, style: Style) -> windows::core::Result<Engine> {
        use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;

        // Lets the brush see what is behind the window. A failure from here on puts it back.
        let accent_before = unsafe { get_accent(hwnd) };
        unsafe {
            set_accent(hwnd, AccentPolicy::with_state(ACCENT_ENABLE_HOSTBACKDROP));
            set_host_backdrop(hwnd, true);
        }
        let undo = Undo { hwnd, accent_before };

        let host = Host::get()?;
        let c = &host.compositor;
        let interop: ICompositorDesktopInterop = c.cast()?;
        // Not topmost: the layers go under the window's content, which is drawn over them.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
        let root = c.CreateContainerVisual()?;
        target.SetRoot(&root)?;

        let props = c.CreatePropertySet()?;
        props.InsertScalar(&hs(SCALE), 1.0)?;
        put_style(&props, &style)?;
        let chain = chain_of(&style);
        let brush = host.brush(&props, chain)?;

        std::mem::forget(undo);
        let mut engine = Engine {
            layers: Vec::new(),
            brush,
            root,
            _target: target,
            props,
            host,
            hwnd,
            style,
            chain,
            regions: None,
            size: (0.0, 0.0),
            dpi_scale: 1.0,
            scale: None,
            shown: true,
            accent_before,
        };
        engine.read_size()?;
        engine.props.InsertScalar(&hs(SCALE), engine.scale())?;
        engine.sync_layers()?;
        engine.refresh()?;
        Ok(engine)
    }

    /// A new engine on the same window with the same state, after DWM restarted.
    pub(crate) fn rebuilt(self) -> windows::core::Result<Engine> {
        let (hwnd, style, regions, scale, shown) = (self.hwnd, self.style, self.regions.clone(), self.scale, self.shown);
        drop(self);
        let mut engine = Engine::new(hwnd, style)?;
        engine.set_scale(scale)?;
        engine.set_regions(regions)?;
        if !shown {
            engine.fade(false, Transition::INSTANT)?;
        }
        Ok(engine)
    }

    fn scale(&self) -> f32 {
        self.scale.unwrap_or(self.dpi_scale)
    }

    // ---- layers

    fn make_layer(&self) -> windows::core::Result<Layer> {
        let c = &self.host.compositor;
        let sprite = c.CreateSpriteVisual()?;
        sprite.SetBrush(&self.brush)?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        let clip = c.CreateGeometricClipWithGeometry(&geometry)?;
        sprite.SetClip(&clip)?;
        Ok(Layer { sprite, clip, shape: Shape::Round { geometry, follows_style: false }, region: None })
    }

    /// One layer per region, or one that fills the window. Layers that are there already are reused.
    fn sync_layers(&mut self) -> windows::core::Result<()> {
        let wanted: Vec<Option<Region>> = match &self.regions {
            None => vec![None],
            Some(regions) => regions.iter().copied().map(Some).collect(),
        };
        let kids = self.root.Children()?;
        while self.layers.len() > wanted.len() {
            if let Some(layer) = self.layers.pop() {
                kids.Remove(&layer.sprite)?;
            }
        }
        let mut layers = std::mem::take(&mut self.layers);
        let had = layers.len();
        let result = (|| {
            for (i, region) in wanted.into_iter().enumerate() {
                if i == layers.len() {
                    let layer = self.make_layer()?;
                    kids.InsertAtTop(&layer.sprite)?;
                    layers.push(layer);
                } else if i < had && layers[i].region == region {
                    continue;
                }
                layers[i].region = region;
                self.place(&mut layers[i], None)?;
            }
            Ok(())
        })();
        self.layers = layers;
        result
    }

    /// Re-places the layers `which` picks.
    fn place_where(&mut self, which: impl Fn(&Layer) -> bool, t: Option<&Transition>) -> windows::core::Result<()> {
        let mut layers = std::mem::take(&mut self.layers);
        let result = layers.iter_mut().filter(|l| which(l)).try_for_each(|l| self.place(l, t));
        self.layers = layers;
        result
    }

    /// Puts a layer where its region says, in its shape. A path corner follows a style change over
    /// `t`; a rounded one follows the property set by itself.
    fn place(&self, layer: &mut Layer, t: Option<&Transition>) -> windows::core::Result<()> {
        let s = self.scale();
        let (x, y, w, h) = match layer.region {
            Some(r) => (r.x * s, r.y * s, r.width * s, r.height * s),
            None => (0.0, 0.0, self.size.0, self.size.1),
        };
        layer.sprite.SetOffset(Vector3 { X: x, Y: y, Z: 0.0 })?;
        layer.sprite.SetSize(Vector2 { X: w, Y: h })?;
        layer.sprite.SetOpacity(layer.region.map_or(1.0, |r| r.opacity))?;

        let corner = layer.region.and_then(|r| r.corner).unwrap_or(self.style.corner);
        let own = layer.region.and_then(|r| r.radius);
        // Paths need Windows 10 1809; where they fail, the corner is round instead.
        if !corner.is_round() && self.draw_path(layer, [w, h, own.unwrap_or(self.style.radius) * s, corner.k()], t).is_ok() {
            return Ok(());
        }
        self.draw_round(layer, Vector2 { X: w, Y: h }, own.map(|r| r * s))
    }

    fn draw_round(&self, layer: &mut Layer, size: Vector2, own: Option<f32>) -> windows::core::Result<()> {
        if !matches!(layer.shape, Shape::Round { .. }) {
            let geometry = self.host.compositor.CreateRoundedRectangleGeometry()?;
            layer.clip.SetGeometry(&geometry)?;
            layer.shape = Shape::Round { geometry, follows_style: false };
        }
        let Shape::Round { geometry, follows_style } = &mut layer.shape else { unreachable!() };
        geometry.SetSize(size)?;
        let corner = hs("CornerRadius");
        match own {
            Some(r) => {
                if *follows_style {
                    geometry.StopAnimation(&corner)?;
                    *follows_style = false;
                }
                geometry.SetCornerRadius(Vector2 { X: r, Y: r })
            }
            None if !*follows_style => {
                geometry.StartAnimation(&corner, &self.host.expression(&self.props, "Vector2(s.Radius, s.Radius) * s.Scale")?)?;
                *follows_style = true;
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn draw_path(&self, layer: &mut Layer, key: [f32; 4], t: Option<&Transition>) -> windows::core::Result<()> {
        if matches!(&layer.shape, Shape::Path { drawn, .. } if *drawn == key) {
            return Ok(());
        }
        let [w, h, r, k] = key;
        let path = CompositionPath::Create(&shape::source(&self.host.d2d()?, shape::outline(w, h, r, k))?)?;
        match &mut layer.shape {
            Shape::Path { geometry, drawn } => {
                let name = hs("Path");
                match t {
                    // Every outline has the same points, so one can be animated into the other.
                    Some(t) => {
                        let a = self.host.compositor.CreatePathKeyFrameAnimation()?;
                        a.InsertKeyFrameWithEasingFunction(1.0, &path, &self.host.easing(t.easing)?)?;
                        time(&a, t)?;
                        geometry.StartAnimation(&name, &a)?;
                    }
                    None => {
                        geometry.StopAnimation(&name)?;
                        geometry.SetPath(&path)?;
                    }
                }
                *drawn = key;
            }
            Shape::Round { .. } => {
                let geometry = self.host.compositor.CreatePathGeometryWithPath(&path)?;
                layer.clip.SetGeometry(&geometry)?;
                layer.shape = Shape::Path { geometry, drawn: key };
            }
        }
        Ok(())
    }

    // ---- what the handle asks for

    pub(crate) fn set_regions(&mut self, regions: Option<Vec<Region>>) -> windows::core::Result<()> {
        let regions = regions.map(|list| list.into_iter().filter_map(Region::sanitized).collect::<Vec<_>>());
        if regions == self.regions {
            return Ok(());
        }
        self.regions = regions;
        self.sync_layers()
    }

    pub(crate) fn set_scale(&mut self, scale: Option<f32>) -> windows::core::Result<()> {
        if scale == self.scale {
            return Ok(());
        }
        self.scale = scale;
        self.props.InsertScalar(&hs(SCALE), self.scale())?;
        self.place_where(|_| true, None)
    }

    /// Moves to `style` over `t`. The compositor runs the animation; nothing is called per frame.
    pub(crate) fn set_style(&mut self, style: Style, t: Transition) -> windows::core::Result<()> {
        let before = self.style;
        self.style = style;
        let moving = !t.is_instant();

        // The brush carries every effect the animation passes through; the ones the end does not
        // need are dropped once it has landed (`settle`).
        let wanted = chain_of(&style);
        self.use_chain(if moving { self.chain.union(wanted) } else { wanted })?;

        let props = &self.props;
        let scalars = [(RADIUS, style.radius), (STRENGTH, style.strength), (SATURATION, style.saturation)];
        if moving {
            let c = &self.host.compositor;
            let ease = self.host.easing(t.easing)?;
            for (name, to) in scalars {
                let a = c.CreateScalarKeyFrameAnimation()?;
                a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
                time(&a, &t)?;
                props.StartAnimation(&hs(name), &a)?;
            }
            let a = c.CreateColorKeyFrameAnimation()?;
            a.SetInterpolationColorSpace(CompositionColorSpace::Rgb)?;
            a.InsertKeyFrameWithEasingFunction(1.0, tint_color(&style), &ease)?;
            time(&a, &t)?;
            props.StartAnimation(&hs(TINT_COLOR), &a)?;
            let ms = (t.delay + t.duration).as_millis().min(u32::MAX as u128 / 2) as u32 + 100;
            unsafe { SetTimer(Some(self.hwnd), SETTLE_TIMER, ms, None) };
        } else {
            for name in [RADIUS, STRENGTH, SATURATION, TINT_COLOR] {
                props.StopAnimation(&hs(name))?;
            }
            put_style(props, &style)?;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), SETTLE_TIMER);
            }
        }

        // Rounded clips follow the property set. Paths, and clips changing shape, are redrawn.
        if before.radius != style.radius || before.corner != style.corner {
            self.place_where(uses_style, moving.then_some(&t))?;
        }
        Ok(())
    }

    /// The animation has landed: effects that do nothing at the end are taken out of the brush.
    pub(crate) fn settle(&mut self) -> windows::core::Result<()> {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), SETTLE_TIMER);
        }
        self.use_chain(chain_of(&self.style))
    }

    fn use_chain(&mut self, chain: Chain) -> windows::core::Result<()> {
        if chain == self.chain {
            return Ok(());
        }
        let brush = self.host.brush(&self.props, chain)?;
        for l in &self.layers {
            l.sprite.SetBrush(&brush)?;
        }
        self.brush = brush;
        self.chain = chain;
        Ok(())
    }

    /// Fades the whole blur in or out.
    pub(crate) fn fade(&mut self, visible: bool, t: Transition) -> windows::core::Result<()> {
        self.shown = visible;
        let to = if visible { 1.0 } else { 0.0 };
        let name = hs("Opacity");
        if t.is_instant() {
            self.root.StopAnimation(&name)?;
            return self.root.SetOpacity(to);
        }
        let a = self.host.compositor.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, to, &self.host.easing(t.easing)?)?;
        time(&a, &t)?;
        self.root.StartAnimation(&name, &a)
    }

    // ---- the window

    fn read_size(&mut self) -> windows::core::Result<()> {
        let mut r = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut r)? };
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        self.dpi_scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        // A minimized window has no client area; it keeps the size it had.
        if w > 0.0 && h > 0.0 {
            self.size = (w, h);
        }
        Ok(())
    }

    /// Catches up with the window's size, DPI and visibility. It stops drawing while the window cannot be seen.
    pub(crate) fn refresh(&mut self) -> windows::core::Result<()> {
        let visible = unsafe { IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() };
        self.root.SetIsVisible(visible)?;
        let (size, scale) = (self.size, self.scale());
        self.read_size()?;
        if self.scale() != scale {
            self.props.InsertScalar(&hs(SCALE), self.scale())?;
            self.place_where(|_| true, None)
        } else if self.size != size {
            self.place_where(|l| l.region.is_none(), None)
        } else {
            Ok(())
        }
    }
}

/// Whether a layer takes its radius or its corner from the style.
fn uses_style(layer: &Layer) -> bool {
    layer.region.map_or(true, |r| r.radius.is_none() || r.corner.is_none())
}

fn put_style(props: &CompositionPropertySet, style: &Style) -> windows::core::Result<()> {
    props.InsertScalar(&hs(RADIUS), style.radius)?;
    props.InsertScalar(&hs(STRENGTH), style.strength)?;
    props.InsertScalar(&hs(SATURATION), style.saturation)?;
    props.InsertColor(&hs(TINT_COLOR), tint_color(style))
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), SETTLE_TIMER);
            restore_accent(self.hwnd, self.accent_before);
            set_host_backdrop(self.hwnd, false);
        }
    }
}

/// Puts the window back if building the engine fails halfway.
struct Undo {
    hwnd: HWND,
    accent_before: Option<AccentPolicy>,
}

impl Drop for Undo {
    fn drop(&mut self) {
        unsafe {
            restore_accent(self.hwnd, self.accent_before);
            set_host_backdrop(self.hwnd, false);
        }
    }
}

/// Windows 11's documented switch for the host backdrop brush. Windows 10 does not know the
/// attribute and refuses it, which is fine: the accent does the job there.
unsafe fn set_host_backdrop(hwnd: HWND, on: bool) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
    let value = on as i32; // a Win32 BOOL
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_HOSTBACKDROPBRUSH,
        &value as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );
}

// ---- accent -----------------------------------------------------------------------------------

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_HOSTBACKDROP: u32 = 5;
const WCA_ACCENT_POLICY: u32 = 19;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

impl AccentPolicy {
    fn with_state(state: u32) -> AccentPolicy {
        AccentPolicy { state, flags: 2, gradient: 0x0100_0000, animation: 0 }
    }
}

#[repr(C)]
struct CompositionAttribData {
    attrib: u32,
    data: *mut std::ffi::c_void,
    size: usize,
}

type CompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttribData) -> i32;

/// `Set`/`GetWindowCompositionAttribute` are undocumented, so they are looked up rather than linked.
unsafe fn composition_attribute(name: windows::core::PCSTR) -> Option<CompositionAttribute> {
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    let user32 = GetModuleHandleA(s!("user32.dll")).ok()?;
    GetProcAddress(user32, name).map(|f| std::mem::transmute::<_, CompositionAttribute>(f))
}

unsafe fn call_accent(name: windows::core::PCSTR, hwnd: HWND, policy: &mut AccentPolicy) -> bool {
    let Some(f) = composition_attribute(name) else { return false };
    let mut data = CompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        data: policy as *mut _ as *mut std::ffi::c_void,
        size: std::mem::size_of::<AccentPolicy>(),
    };
    f(hwnd, &mut data) != 0
}

unsafe fn set_accent(hwnd: HWND, mut policy: AccentPolicy) {
    call_accent(windows::core::s!("SetWindowCompositionAttribute"), hwnd, &mut policy);
}

/// The accent the window had before, to give it back on detach.
unsafe fn get_accent(hwnd: HWND) -> Option<AccentPolicy> {
    let mut policy = AccentPolicy::default();
    call_accent(windows::core::s!("GetWindowCompositionAttribute"), hwnd, &mut policy).then_some(policy)
}

unsafe fn restore_accent(hwnd: HWND, before: Option<AccentPolicy>) {
    match before {
        Some(p) if p.state != ACCENT_ENABLE_HOSTBACKDROP => set_accent(hwnd, p),
        _ => set_accent(hwnd, AccentPolicy::with_state(ACCENT_DISABLED)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chain_builds_and_binds() {
        let host = Host::get().unwrap();
        let props = host.compositor.CreatePropertySet().unwrap();
        props.InsertScalar(&hs(SCALE), 1.5).unwrap();
        put_style(&props, &Style::frost().tint(0.3)).unwrap();
        for chain in Chain::all() {
            host.brush(&props, chain).unwrap_or_else(|e| panic!("chain {chain:?}: {e}"));
        }
    }

    #[test]
    fn a_squircle_reaches_composition_and_animates() {
        let host = Host::get().unwrap();
        let c = &host.compositor;
        let d2d = host.d2d().unwrap();
        let path = |r| CompositionPath::Create(&shape::source(&d2d, shape::outline(200.0, 80.0, r, 2.0)).unwrap()).unwrap();
        let geometry = c.CreatePathGeometryWithPath(&path(12.0)).unwrap();
        let a = c.CreatePathKeyFrameAnimation().unwrap();
        a.InsertKeyFrameWithEasingFunction(1.0, &path(30.0), &host.easing(Easing::DEFAULT).unwrap()).unwrap();
        time(&a, &Transition::new(Duration::from_millis(100))).unwrap();
        geometry.StartAnimation(&hs("Path"), &a).unwrap();
    }

    #[test]
    fn the_chain_leaves_out_what_does_nothing() {
        assert_eq!(chain_of(&Style::default().strength(0.0)), Chain::NONE);
        assert_eq!(chain_of(&Style::default()), Chain::BLUR);
        assert!(chain_of(&Style::frost()).has(Chain::SATURATION));
        assert!(chain_of(&Style::dark()).has(Chain::TINT));
    }
}
