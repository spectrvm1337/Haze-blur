# Haze

![A Haze window gliding over an animated wallpaper and changing its radius, blur strength and tint](assets/demo.gif)

Rounded backdrop blur for Windows 10 windows, in Rust, with a Tauri plugin and an npm package for Electron.

Windows 10 blurs only whole rectangles behind a window. Put a rounded card in a transparent window and the blur shows square corners around it. Haze draws the blur itself with `Windows.UI.Composition`, so it takes the card's shape. The radius, the strength and the tint can be changed at any time, and the compositor animates the change.

## Use

```toml
[dependencies]
haze = "0.3"
```

```rust
use std::time::Duration;
use haze::{Haze, Style};

// On the thread that owns the window.
let haze = Haze::attach(&window, Style::dark().radius(20.0))?;

// Later, from any thread. The compositor runs the animation.
haze.animate_to(Style::light().radius(32.0), Duration::from_millis(300));
haze.fade(false, Duration::from_millis(200));
```

`attach` takes anything with a window handle, and `attach_hwnd` takes a raw `HWND`. After that the blur follows the window's size and DPI, and it stops drawing while the window is hidden or minimized.

The blur belongs to the window. It goes away when the window is destroyed, or when you call `haze.detach()`. A `Haze` is a handle: cloning is cheap, it can be sent to other threads, and dropping it does not remove the blur.

### What the window needs

The window has to be transparent and must not keep a surface of its own, or white shows outside the rounded corners. In Win32 that is `WS_EX_NOREDIRECTIONBITMAP`. In winit it is `with_transparent(true)` plus `with_no_redirection_bitmap(true)`. Transparent Tauri and Electron windows are fine as they are.

## Tauri

The plugin in `tauri-plugin-haze` drives the blur from the page:

```rust
tauri::Builder::default().plugin(tauri_plugin_haze::init())
```

Add `"haze:default"` to the window's capability and make the window and the page background transparent. Then:

```js
await invoke('plugin:haze|attach', { style: { preset: 'dark', radius: 20 } });
await invoke('plugin:haze|animate_to', { style: { preset: 'frost', radius: 32 }, ms: 300 });
await invoke('plugin:haze|fade', { visible: false, ms: 200 });
await invoke('plugin:haze|set_regions', { regions: [{ x: 0, y: 0, width: 240, height: 700, radius: 16 }] });
await invoke('plugin:haze|detach');
```

`examples/tauri` is a complete app. From Rust, `Haze::attach(&window, style)` also works in `setup`.

## Electron

The package in `npm` calls `haze.dll` through [koffi](https://koffi.dev). Run `npm run build` in that folder to build the DLL, which needs Rust. Then, from the main process:

```js
const haze = require('haze-blur');

const win = new BrowserWindow({ transparent: true, frame: false });
const blur = haze.attach(win, { preset: 'dark', radius: 20 });
blur.animateTo({ preset: 'frost', radius: 32 }, 300);
```

The methods are `setStyle`, `animateTo`, `fade`, `setRegions` and `detach`.

## Other languages

`cargo build --release --features ffi` builds `haze.dll` for C#, Python or anything else that loads a DLL. The functions are declared at the top of `src/ffi.rs`.

## Style

```rust
Style::dark()    // graphite tint
Style::light()
Style::frost()   // strong blur, a trace of white, stronger colors
Style::default() // plain blur, no tint

Style::dark().radius(24.0).strength(18.0).tint(0.5).tint_color([10, 10, 14]).saturation(1.2)
```

`radius` is in CSS pixels and is scaled by the window's DPI. `strength` is extra Gaussian blur on top of the system's own. `tint` is how much of `tint_color` lies over the blur, from 0 to 1. A blur over a light backdrop is hard to see without it. `saturation` of 1 leaves the colors alone.

## Regions

The blur can cover parts of the window instead of all of it, for example a sidebar and a card:

```rust
use haze::Region;

haze.set_regions(&[
    Region::new(0.0, 0.0, 240.0, 700.0),
    Region::new(260.0, 20.0, 200.0, 48.0).radius(24.0), // half the height makes a pill
]);
haze.set_regions(&[]); // back to the whole window
```

Regions are in CSS pixels from the top-left corner of the client area and stay put when the window is resized.

## Examples

```
cargo run --example demo    # a bare Win32 window
cargo run --example winit
```

In the demo the blur changes style every couple of seconds. Drag the window by its body, press Space to fade the blur out and in, D to switch between dark and light, Esc to close.

## How it works

1. `SetWindowCompositionAttribute` with accent state 5 (`ACCENT_ENABLE_HOSTBACKDROP`) lets a `HostBackdropBrush` see what is behind the window. The call is undocumented. Haze also sets `DWMWA_USE_HOSTBACKDROPBRUSH`, the documented switch on Windows 11.
2. A `Compositor` gets a `DesktopWindowTarget` on the window. It is not topmost, so the window's own content draws over it.
3. One sprite, painted with the `HostBackdropBrush` through saturation and Gaussian blur effects, is cut to a rounded rectangle with a `CompositionGeometricClip`. A second sprite with a color brush lies over it as the tint, cut the same way.
4. The effects are small hand-written `IGraphicsEffect` objects describing Direct2D effects, because Win2D does not run in desktop apps. Their amounts are named properties, so changing them needs no new effect.
5. The window procedure is replaced with `SetWindowLongPtr` to catch size, DPI and visibility changes. `SetWindowSubclass` would need comctl32 v6, which an exe gets only through a manifest.

## Status

- Tested on Windows 10 22H2 (build 19045) in a plain Win32 window, winit, Tauri 2 and Electron 45.
- Not tested on Windows 11 or on other builds of Windows 10.
- Not tested at display scales above 100%.
- Acrylic and blur accents are known to make windows laggy while dragging on 1903 and newer. Haze has not been measured with a real mouse drag.
- `HostBackdropBrush` needs Windows 10 1803 or newer. `Haze::supported()` tells you whether it is there.
- A Windows update can break the undocumented call.

`cargo test --features ffi` runs the blur on real windows, so it needs a desktop session.

## Related

- [window-vibrancy](https://github.com/tauri-apps/window-vibrancy) gives Tauri windows blur, acrylic and mica, rectangular.
- [Win32Acrylic](https://github.com/ALTaleX531/Win32Acrylic) draws acrylic through Composition in C++, over the whole window and without a clip.
- A [Microsoft Q&A thread](https://learn.microsoft.com/en-us/answers/a/152907) on blurring behind a Win32 or WPF window with the Visual Layer.

## License

MIT
