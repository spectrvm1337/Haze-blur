//! Haze for Tauri 2: the rounded backdrop blur, controlled from JavaScript. Windows only.
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
#![cfg(windows)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use haze::{Haze, Region, Style};
use serde::Deserialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, State, WebviewWindow};

/// The blur of each window, by window label.
#[derive(Default)]
struct Hazes(Mutex<HashMap<String, Haze>>);

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
}

impl StyleArgs {
    fn build(self) -> Style {
        let mut style = match self.preset.as_deref() {
            Some("dark") => Style::dark(),
            Some("light") => Style::light(),
            Some("frost") => Style::frost(),
            _ => Style::default(),
        };
        if let Some(v) = self.radius {
            style = style.radius(v);
        }
        if let Some(v) = self.strength {
            style = style.strength(v);
        }
        if let Some(v) = self.tint {
            style = style.tint(v);
        }
        if let Some(v) = self.tint_color {
            style = style.tint_color(v);
        }
        if let Some(v) = self.saturation {
            style = style.saturation(v);
        }
        style
    }
}

#[derive(Deserialize)]
struct RegionArgs {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: Option<f32>,
}

fn haze_of<R: Runtime>(window: &WebviewWindow<R>, hazes: &State<'_, Hazes>) -> Result<Haze, String> {
    hazes.0.lock().unwrap().get(window.label()).cloned().ok_or_else(|| "no blur on this window, call attach first".to_string())
}

// Commands that are not async run on the main thread, which is the one that owns the window.
#[tauri::command]
fn attach<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, style: Option<StyleArgs>) -> Result<(), String> {
    let style = style.unwrap_or_default().build();
    let mut map = hazes.0.lock().unwrap();
    match Haze::attach(&window, style) {
        Ok(haze) => {
            map.insert(window.label().to_string(), haze);
            Ok(())
        }
        Err(e) => match map.get(window.label()) {
            // Attached earlier, the page was only reloaded.
            Some(haze) => {
                haze.set_style(style);
                Ok(())
            }
            None => Err(e.to_string()),
        },
    }
}

#[tauri::command]
fn set_style<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, style: StyleArgs) -> Result<(), String> {
    haze_of(&window, &hazes)?.set_style(style.build());
    Ok(())
}

#[tauri::command]
fn animate_to<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, style: StyleArgs, ms: u64) -> Result<(), String> {
    haze_of(&window, &hazes)?.animate_to(style.build(), Duration::from_millis(ms));
    Ok(())
}

#[tauri::command]
fn fade<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, visible: bool, ms: u64) -> Result<(), String> {
    haze_of(&window, &hazes)?.fade(visible, Duration::from_millis(ms));
    Ok(())
}

#[tauri::command]
fn set_regions<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>, regions: Vec<RegionArgs>) -> Result<(), String> {
    let regions: Vec<Region> = regions
        .into_iter()
        .map(|r| {
            let region = Region::new(r.x, r.y, r.width, r.height);
            match r.radius {
                Some(v) => region.radius(v),
                None => region,
            }
        })
        .collect();
    haze_of(&window, &hazes)?.set_regions(&regions);
    Ok(())
}

#[tauri::command]
fn detach<R: Runtime>(window: WebviewWindow<R>, hazes: State<'_, Hazes>) -> Result<(), String> {
    haze_of(&window, &hazes)?.detach();
    hazes.0.lock().unwrap().remove(window.label());
    Ok(())
}

/// The plugin. Add it to the Tauri builder.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("haze")
        .invoke_handler(tauri::generate_handler![attach, set_style, animate_to, fade, set_regions, detach])
        .setup(|app, _| {
            app.manage(Hazes::default());
            Ok(())
        })
        .build()
}
