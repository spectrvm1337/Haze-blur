//! Haze for Tauri 2: the rounded backdrop blur, controlled from JavaScript.
//!
//! ```ignore
//! tauri::Builder::default().plugin(tauri_plugin_haze::init())
//! ```
//!
//! Add `"haze:default"` to the window's capability, make the window transparent, then from the page:
//!
//! ```js
//! await invoke('plugin:haze|attach', { style: { preset: 'dark', radius: 20 } });
//! await invoke('plugin:haze|animate_to', { style: { preset: 'frost', radius: 32 }, ms: 300 });
//! ```
//!
//! The blur is Windows only. On other systems the plugin loads and its commands do nothing, and
//! `supported` answers false, so a cross-platform app can register it unconditionally.

use serde::Deserialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, State, WebviewWindow};

/// A style as the page sends it: a preset, and whatever differs from it.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StyleArgs {
    preset: Option<String>,
    radius: Option<f32>,
    strength: Option<f32>,
    tint: Option<f32>,
    tint_color: Option<[u8; 3]>,
    saturation: Option<f32>,
    corner: Option<CornerArg>,
}

/// `"round"`, `"squircle"`, or the `k` of CSS `superellipse(k)`.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum CornerArg {
    K(f32),
    Name(CornerName),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CornerName {
    Round,
    Squircle,
}

#[derive(Deserialize)]
struct RegionArgs {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: Option<f32>,
    opacity: Option<f32>,
    corner: Option<CornerArg>,
}

/// `"linear"` or the four numbers of CSS `cubic-bezier()`.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum EasingArg {
    Bezier([f32; 4]),
    Name(EasingName),
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum EasingName {
    Linear,
}

#[cfg(windows)]
mod blur {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use haze::{Corner, Easing, Haze, Region, Style, Transition};
    use tauri::{Runtime, WebviewWindow};

    use super::{CornerArg, CornerName, EasingArg, EasingName, RegionArgs, StyleArgs};

    /// The blur of each window, by window label.
    #[derive(Default)]
    pub struct Hazes(Mutex<HashMap<String, Haze>>);

    impl Hazes {
        fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Haze>> {
            self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        fn of(&self, label: &str) -> Result<Haze, String> {
            self.lock().get(label).filter(|h| h.is_attached()).cloned().ok_or_else(|| "no blur on this window, call attach first".into())
        }
        pub fn forget(&self, label: &str) {
            self.lock().remove(label);
        }
    }

    fn corner(c: CornerArg) -> Corner {
        match c {
            CornerArg::Name(CornerName::Round) => Corner::Round,
            CornerArg::Name(CornerName::Squircle) => Corner::Squircle,
            CornerArg::K(k) => Corner::Superellipse(k),
        }
    }

    fn style(args: StyleArgs) -> Style {
        let mut s = match args.preset.as_deref() {
            Some("dark") => Style::dark(),
            Some("light") => Style::light(),
            Some("frost") => Style::frost(),
            _ => Style::default(),
        };
        if let Some(v) = args.radius {
            s = s.radius(v);
        }
        if let Some(v) = args.strength {
            s = s.strength(v);
        }
        if let Some(v) = args.tint {
            s = s.tint(v);
        }
        if let Some(v) = args.tint_color {
            s = s.tint_color(v);
        }
        if let Some(v) = args.saturation {
            s = s.saturation(v);
        }
        if let Some(c) = args.corner {
            s = s.corner(corner(c));
        }
        s
    }

    fn transition(ms: u64, delay: Option<u64>, easing: Option<EasingArg>) -> Transition {
        let t = Transition::new(Duration::from_millis(ms)).delay(Duration::from_millis(delay.unwrap_or(0)));
        match easing {
            Some(EasingArg::Bezier(b)) => t.easing(Easing::CubicBezier(b)),
            Some(EasingArg::Name(EasingName::Linear)) => t.easing(Easing::Linear),
            None => t,
        }
    }

    pub fn attach<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes, args: Option<StyleArgs>) -> Result<(), String> {
        let style = style(args.unwrap_or_default());
        let mut map = hazes.lock();
        if let Some(haze) = map.get(window.label()).filter(|h| h.is_attached()) {
            // Attached earlier and the page was reloaded: start over from a clean blur.
            haze.cover_window();
            haze.set_scale(None);
            haze.fade(true, Transition::INSTANT);
            haze.set_style(style);
            return Ok(());
        }
        let haze = Haze::attach(window, style).map_err(|e| e.to_string())?;
        map.insert(window.label().to_string(), haze);
        Ok(())
    }

    pub fn set_style<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes, args: StyleArgs) -> Result<(), String> {
        hazes.of(window.label())?.set_style(style(args));
        Ok(())
    }

    pub fn animate_to<R: Runtime>(
        window: &WebviewWindow<R>,
        hazes: &Hazes,
        args: StyleArgs,
        ms: u64,
        delay: Option<u64>,
        easing: Option<EasingArg>,
    ) -> Result<(), String> {
        hazes.of(window.label())?.animate_to(style(args), transition(ms, delay, easing));
        Ok(())
    }

    pub fn fade<R: Runtime>(
        window: &WebviewWindow<R>,
        hazes: &Hazes,
        visible: bool,
        ms: u64,
        delay: Option<u64>,
        easing: Option<EasingArg>,
    ) -> Result<(), String> {
        hazes.of(window.label())?.fade(visible, transition(ms, delay, easing));
        Ok(())
    }

    pub fn set_regions<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes, regions: Vec<RegionArgs>) -> Result<(), String> {
        let regions: Vec<Region> = regions
            .into_iter()
            .map(|r| {
                let mut region = Region::new(r.x, r.y, r.width, r.height);
                if let Some(v) = r.radius {
                    region = region.radius(v);
                }
                if let Some(v) = r.opacity {
                    region = region.opacity(v);
                }
                if let Some(c) = r.corner {
                    region = region.corner(corner(c));
                }
                region
            })
            .collect();
        hazes.of(window.label())?.set_regions(&regions);
        Ok(())
    }

    pub fn cover_window<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes) -> Result<(), String> {
        hazes.of(window.label())?.cover_window();
        Ok(())
    }

    pub fn set_scale<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes, scale: Option<f32>) -> Result<(), String> {
        hazes.of(window.label())?.set_scale(scale);
        Ok(())
    }

    pub fn detach<R: Runtime>(window: &WebviewWindow<R>, hazes: &Hazes) -> Result<(), String> {
        if let Some(haze) = hazes.lock().remove(window.label()) {
            haze.detach();
        }
        Ok(())
    }

    pub fn supported() -> bool {
        Haze::supported()
    }
}

/// Elsewhere the commands are accepted and do nothing.
#[cfg(not(windows))]
mod blur {
    use tauri::{Runtime, WebviewWindow};

    use super::{EasingArg, RegionArgs, StyleArgs};

    #[derive(Default)]
    pub struct Hazes;

    impl Hazes {
        pub fn forget(&self, _label: &str) {}
    }

    pub fn attach<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes, _: Option<StyleArgs>) -> Result<(), String> {
        Ok(())
    }
    pub fn set_style<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes, _: StyleArgs) -> Result<(), String> {
        Ok(())
    }
    pub fn animate_to<R: Runtime>(
        _: &WebviewWindow<R>,
        _: &Hazes,
        _: StyleArgs,
        _: u64,
        _: Option<u64>,
        _: Option<EasingArg>,
    ) -> Result<(), String> {
        Ok(())
    }
    pub fn fade<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes, _: bool, _: u64, _: Option<u64>, _: Option<EasingArg>) -> Result<(), String> {
        Ok(())
    }
    pub fn set_regions<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes, _: Vec<RegionArgs>) -> Result<(), String> {
        Ok(())
    }
    pub fn cover_window<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes) -> Result<(), String> {
        Ok(())
    }
    pub fn set_scale<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes, _: Option<f32>) -> Result<(), String> {
        Ok(())
    }
    pub fn detach<R: Runtime>(_: &WebviewWindow<R>, _: &Hazes) -> Result<(), String> {
        Ok(())
    }
    pub fn supported() -> bool {
        false
    }
}

use blur::Hazes;

// Commands that are not async run on the main thread, which is the one that owns the windows.
#[tauri::command]
fn attach<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, style: Option<StyleArgs>) -> Result<(), String> {
    blur::attach(&window, &hazes, style)
}

#[tauri::command]
fn set_style<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, style: StyleArgs) -> Result<(), String> {
    blur::set_style(&window, &hazes, style)
}

#[tauri::command]
fn animate_to<R: Runtime>(
    window: WebviewWindow<R>,
    hazes: State<'_, Hazes>,
    style: StyleArgs,
    ms: u64,
    delay: Option<u64>,
    easing: Option<EasingArg>,
) -> Result<(), String> {
    blur::animate_to(&window, &hazes, style, ms, delay, easing)
}

#[tauri::command]
fn fade<R: Runtime>(
    window: WebviewWindow<R>,
    hazes: State<'_, Hazes>,
    visible: bool,
    ms: Option<u64>,
    delay: Option<u64>,
    easing: Option<EasingArg>,
) -> Result<(), String> {
    blur::fade(&window, &hazes, visible, ms.unwrap_or(0), delay, easing)
}

#[tauri::command]
fn set_regions<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, regions: Vec<RegionArgs>) -> Result<(), String> {
    blur::set_regions(&window, &hazes, regions)
}

#[tauri::command]
fn cover_window<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>) -> Result<(), String> {
    blur::cover_window(&window, &hazes)
}

#[tauri::command]
fn set_scale<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, scale: Option<f32>) -> Result<(), String> {
    blur::set_scale(&window, &hazes, scale)
}

#[tauri::command]
fn detach<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>) -> Result<(), String> {
    blur::detach(&window, &hazes)
}

#[tauri::command]
fn supported() -> bool {
    blur::supported()
}

/// The plugin. Add it to the Tauri builder.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("haze")
        .invoke_handler(tauri::generate_handler![
            attach,
            set_style,
            animate_to,
            fade,
            set_regions,
            cover_window,
            set_scale,
            detach,
            supported
        ])
        .setup(|app, _| {
            app.manage(Hazes::default());
            Ok(())
        })
        .on_event(|app, event| {
            // A window that is closed takes its blur with it; a new window under the same label starts fresh.
            if let tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::Destroyed, .. } = event {
                app.state::<Hazes>().forget(label);
            }
        })
        .build()
}
