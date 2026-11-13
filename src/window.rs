//! The window side: a subclass that keeps the engine on the window's thread, tells it when the
//! window changes, and carries out what other threads ask for.
//!
//! The subclass is `SetWindowSubclass` from comctl32, so it chains properly with other code that
//! subclasses the same window, in any order. comctl32 5.82, which programs without a manifest get,
//! exports it by ordinal only, so it is looked up by ordinal.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use windows::core::{w, Error, Result, PCSTR};
use windows::Win32::Foundation::{E_NOTIMPL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetWindowLongPtrW, RegisterWindowMessageW, GWL_EXSTYLE, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_DWMCOMPOSITIONCHANGED,
    WM_NCDESTROY, WM_SIZE, WM_TIMER, WM_WINDOWPOSCHANGED, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP,
};

use crate::engine::{Engine, SETTLE_TIMER};
use crate::{lock, Pending, Shared};

/// Our subclass among the window's others.
const SUBCLASS_ID: usize = 0x4841_5A45;

type SubclassProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
type Export = unsafe extern "system" fn() -> isize;
type SetWindowSubclass = unsafe extern "system" fn(HWND, SubclassProc, usize, usize) -> i32;
type GetWindowSubclass = unsafe extern "system" fn(HWND, SubclassProc, usize, *mut usize) -> i32;
type RemoveWindowSubclass = unsafe extern "system" fn(HWND, SubclassProc, usize) -> i32;
type DefSubclassProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

struct Comctl {
    set: SetWindowSubclass,
    get: GetWindowSubclass,
    remove: RemoveWindowSubclass,
    def: DefSubclassProc,
}

fn comctl() -> Result<&'static Comctl> {
    use std::mem::transmute;
    static COMCTL: OnceLock<Option<Comctl>> = OnceLock::new();
    COMCTL
        .get_or_init(|| unsafe {
            // Kept loaded for good: the window procedures live in it.
            let module = LoadLibraryW(w!("comctl32.dll")).ok()?;
            let at = |ordinal: usize| GetProcAddress(module, PCSTR(ordinal as *const u8));
            Some(Comctl {
                set: transmute::<Export, SetWindowSubclass>(at(410)?),
                get: transmute::<Export, GetWindowSubclass>(at(411)?),
                remove: transmute::<Export, RemoveWindowSubclass>(at(412)?),
                def: transmute::<Export, DefSubclassProc>(at(413)?),
            })
        })
        .as_ref()
        .ok_or_else(|| Error::new(E_NOTIMPL, "comctl32 has no window subclassing"))
}

/// The message that tells the window's thread there is work waiting. Registered rather than taken
/// from the `WM_APP` range, which belongs to the program.
pub(crate) fn wm_haze() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| match unsafe { RegisterWindowMessageW(w!("Haze.Run.4841_5A45")) } {
        0 => WM_APP + 0x4A5,
        m => m,
    })
}

/// What the subclass keeps for the window. The subclass holds one reference; every call into it
/// takes another, so a detach inside a call does not free what that call is still using.
pub(crate) struct Slot {
    hwnd: HWND,
    pub(crate) shared: Arc<Shared>,
    engine: RefCell<Option<Engine>>,
    installed: Cell<bool>,
}

/// Subclasses the window and hands it the engine. On the window's thread.
pub(crate) fn install(hwnd: HWND, shared: Arc<Shared>, engine: Engine) -> Result<()> {
    let c = comctl()?;
    let slot = Rc::new(Slot { hwnd, shared, engine: RefCell::new(Some(engine)), installed: Cell::new(true) });
    let data = Rc::into_raw(slot) as usize;
    if unsafe { (c.set)(hwnd, subclass_proc, SUBCLASS_ID, data) } == 0 {
        // The engine goes with the slot and puts the window back as it was.
        drop(unsafe { Rc::from_raw(data as *const Slot) });
        return Err(Error::from_win32());
    }
    Ok(())
}

/// The window's slot, if the blur is on it. On the window's thread.
pub(crate) fn find(hwnd: HWND) -> Option<Rc<Slot>> {
    let c = comctl().ok()?;
    let mut data = 0usize;
    if unsafe { (c.get)(hwnd, subclass_proc, SUBCLASS_ID, &mut data) } == 0 || data == 0 {
        return None;
    }
    unsafe {
        Rc::increment_strong_count(data as *const Slot);
        Some(Rc::from_raw(data as *const Slot))
    }
}

impl Slot {
    /// Carries out what has been asked.
    pub(crate) fn run(&self) {
        self.shared.posted.store(false, Ordering::Release);
        let pending = std::mem::take(&mut *lock(&self.shared.pending));
        if pending.is_empty() {
            return;
        }
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            // Called from inside the engine's own work: put it back for the message loop.
            let mut queue = lock(&self.shared.pending);
            *queue = pending.then(std::mem::take(&mut *queue));
            drop(queue);
            self.shared.post();
            return;
        };
        if pending.detach {
            drop(engine);
            self.uninstall();
            return;
        }
        if let Some(e) = engine.as_mut() {
            self.shared.record(apply(e, pending));
        }
    }

    fn with_engine(&self, f: impl FnOnce(&mut Engine) -> Result<()>) {
        if let Ok(mut engine) = self.engine.try_borrow_mut() {
            if let Some(e) = engine.as_mut() {
                self.shared.record(f(e));
            }
        }
    }

    /// Lets go of the compositor while the window is still whole: tearing it down when the window
    /// is half gone crashed the process.
    fn drop_engine(&self) {
        self.shared.alive.store(false, Ordering::Release);
        if let Ok(mut engine) = self.engine.try_borrow_mut() {
            engine.take();
        }
    }

    /// Takes the blur off and the subclass out. The caller holds a reference, so `self` stays valid.
    fn uninstall(&self) {
        self.drop_engine();
        if self.installed.replace(false) {
            if let Ok(c) = comctl() {
                unsafe { (c.remove)(self.hwnd, subclass_proc, SUBCLASS_ID) };
            }
            unsafe { Rc::decrement_strong_count(self as *const Slot) };
        }
    }

    /// DWM restarted (a driver update or crash): what it knew about the window is gone, so the
    /// window's transparency and the blur are set up again.
    fn recover(&self) {
        restore_transparency(self.hwnd);
        let Ok(mut engine) = self.engine.try_borrow_mut() else { return };
        if let Some(old) = engine.take() {
            match old.rebuilt() {
                Ok(e) => *engine = Some(e),
                Err(e) => self.shared.record(Err(e)),
            }
        }
    }
}

fn apply(e: &mut Engine, p: Pending) -> Result<()> {
    let mut result = Ok(());
    let mut keep = |r: Result<()>| {
        if r.is_err() {
            result = r;
        }
    };
    if let Some(scale) = p.scale {
        keep(e.set_scale(scale));
    }
    if let Some(regions) = p.regions {
        keep(e.set_regions(regions));
    }
    if let Some((style, t)) = p.style {
        keep(e.set_style(style, t));
    }
    if let Some((visible, t)) = p.fade {
        keep(e.fade(visible, t));
    }
    result
}

/// Windows made transparent with `DwmEnableBlurBehindWindow` and an empty region (winit and tao, and
/// so Tauri, do this) lose that when DWM restarts and show black where they were clear. A window
/// with no redirection bitmap or a layered one never had it and is left alone.
fn restore_transparency(hwnd: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex & (WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_LAYERED.0) != 0 {
            return;
        }
        let region = CreateRectRgn(0, 0, -1, -1);
        let blur = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: true.into(),
            hRgnBlur: region,
            fTransitionOnMaximized: false.into(),
        };
        let _ = DwmEnableBlurBehindWindow(hwnd, &blur);
        let _ = DeleteObject(region.into());
    }
}

unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, data: usize) -> LRESULT {
    let Ok(c) = comctl() else { return DefWindowProcW(hwnd, msg, wp, lp) };
    Rc::increment_strong_count(data as *const Slot);
    let slot = Rc::from_raw(data as *const Slot);
    let forward = || (c.def)(hwnd, msg, wp, lp);

    if msg == wm_haze() {
        slot.run();
        return LRESULT(0);
    }
    match msg {
        WM_TIMER if wp.0 == SETTLE_TIMER => {
            slot.with_engine(Engine::settle);
            LRESULT(0)
        }
        // After the window's own handling, which may resize it (a DPI change does).
        WM_WINDOWPOSCHANGED | WM_SIZE | WM_DPICHANGED => {
            let r = forward();
            slot.with_engine(Engine::refresh);
            r
        }
        WM_DWMCOMPOSITIONCHANGED => {
            let r = forward();
            slot.recover();
            r
        }
        WM_DESTROY => {
            slot.drop_engine();
            forward()
        }
        WM_NCDESTROY => {
            slot.uninstall();
            forward()
        }
        _ => forward(),
    }
}
