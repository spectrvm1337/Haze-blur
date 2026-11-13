# Haze

![A Haze window gliding over an animated wallpaper and changing its radius, blur strength and tint](assets/demo.gif)

Rounded backdrop blur for Windows 10 windows, in Rust, with a Tauri plugin and an npm package for Electron.

Windows 10 blurs only whole rectangles behind a window. Put a rounded card in a transparent window and the blur shows square corners around it. Haze draws the blur itself with `Windows.UI.Composition`, so it takes the card's shape, round or squircle. The radius, the strength and the tint can be changed at any time, and the compositor animates the change.

## Use

```toml
[dependencies]
haze = "0.4"
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

The blur belongs to the window. It goes away when the window is destroyed, or when you call `haze.detach()`, and it can be attached again after that. A `Haze` is a handle: cloning is cheap, it can be sent to other threads, and dropping it does not remove the blur. `is_attached()` tells whether the blur is still there, and `last_error()` returns the last error the compositor reported, since the calls themselves return nothing.

### What the window needs

The window has to be transparent, or nothing shows through. A window without a surface of its own is best, because nothing is drawn outside the rounded corners. In Win32 that is `WS_EX_NOREDIRECTIONBITMAP`. In winit it is `with_transparent(true)` plus `with_no_redirection_bitmap(true)`. Transparent Tauri and Electron windows are fine as they are.

## Style

```rust
use haze::{Corner, Style};

Style::dark()    // graphite tint
Style::light()
Style::frost()   // strong blur, a trace of white, stronger colors
Style::default() // plain blur, no tint

Style::dark().radius(24.0).strength(18.0).tint(0.5).tint_color([10, 10, 14]).saturation(1.2).corner(Corner::Squircle)
```

`radius` and `strength` are in CSS pixels and are scaled by the window's DPI. `tint` is how much of `tint_color` lies over the blur, from 0 to 1. A blur over a light backdrop is hard to see without it. `saturation` of 1 leaves the colors alone. `corner` is `Round`, `Squircle`, or any CSS `superellipse(k)` as `Corner::Superellipse(k)`.

An effect at its neutral value costs nothing: with no tint, saturation 1 and strength 0 the window shows the system's backdrop with no effect at all.

## Regions

The blur can cover parts of the window instead of all of it, for example a sidebar and a card:

```rust
use haze::{Corner, Region};

haze.set_regions(&[
    Region::new(0.0, 0.0, 240.0, 700.0),
    Region::new(260.0, 20.0, 200.0, 48.0).radius(24.0), // half the height makes a pill
    Region::new(260.0, 80.0, 200.0, 120.0).corner(Corner::Squircle).opacity(0.6),
]);
haze.set_regions(&[]); // nothing
haze.cover_window();   // the whole window again
```

Regions are in CSS pixels from the top-left corner of the client area and stay put when the window is resized. Each one can have its own radius, corner and opacity. Sending them every frame, for example from a page whose cards move, is fine: regions that did not change cost nothing, and the rest are updated in place.

Units follow the window's DPI. `haze.set_scale(Some(1.0))` switches to physical pixels, and `Some(devicePixelRatio)` keeps up with a zoomed webview.

## Transitions

`animate_to` and `fade` take a `Duration` or a `Transition`:

```rust
use haze::{Easing, Transition};

haze.animate_to(style, Transition::new(Duration::from_millis(300))
    .delay(Duration::from_millis(33))              // wait for the webview's frame to land
    .easing(Easing::CubicBezier([0.2, 0.0, 0.0, 1.0])));
```

The default curve is `cubic-bezier(0.32, 0.72, 0, 1)`. An animation can be replaced at any time and starts from where the last one is.

## Tauri

The plugin in `tauri-plugin-haze` drives the blur from the page:

```rust
tauri::Builder::default().plugin(tauri_plugin_haze::init())
```

Add `"haze:default"` to the window's capability and make the window and the page background transparent. Then, with the small JS package in `tauri-plugin-haze/guest-js`:

```js
import * as haze from 'tauri-plugin-haze-api';

await haze.attach({ preset: 'dark', radius: 20 });
await haze.animateTo({ preset: 'frost', radius: 32, corner: 'squircle' }, 300);
await haze.fade(false, 200);
await haze.setRegions([{ x: 0, y: 0, width: 240, height: 700, radius: 16 }]);
await haze.detach();
```

or `invoke('plugin:haze|attach', { style: { preset: 'dark' } })` and so on. On macOS and Linux the plugin loads and does nothing, and `supported()` answers false. `examples/tauri` is a complete app. From Rust, `Haze::attach(&window, style)` also works in `setup`.

## Electron

The package in `npm` calls `haze.dll` through [koffi](https://koffi.dev). Run `npm run build` in that folder to build the DLL, which needs Rust. Then, from the main process:

```js
const haze = require('haze-blur');

const win = new BrowserWindow({ transparent: true, frame: false });
const blur = haze.attach(win, { preset: 'dark', radius: 20 });
blur.animateTo({ preset: 'frost', radius: 32 }, 300);
```

The methods are `setStyle`, `animateTo`, `fade`, `setRegions`, `coverWindow`, `setScale`, `isAttached` and `detach`. In a packaged app, unpack the DLL from the asar archive: `"asarUnpack": ["node_modules/haze-blur/bin/**"]` in electron-builder.

## Other languages

`cargo build --release --features ffi` builds `haze.dll` for C#, Python or anything else that loads a DLL. The functions are declared at the top of `src/ffi.rs`.

## Examples

```
cargo run --example demo    # a bare Win32 window
cargo run --example winit
```

In the demo the blur changes style every couple of seconds. Drag the window by its body, press Space to fade the blur out and in, D to switch between dark and light, S to switch between round and squircle corners, Esc to close.

## How it works

1. `SetWindowCompositionAttribute` with accent state 5 (`ACCENT_ENABLE_HOSTBACKDROP`) lets a `HostBackdropBrush` see what is behind the window. The call is undocumented. Haze also sets `DWMWA_USE_HOSTBACKDROPBRUSH`, the documented switch on Windows 11. On detach the window gets its old accent back.
2. A `Compositor`, one per thread, gets a `DesktopWindowTarget` on the window. It is not topmost, so the window's own content draws over it.
3. Each region is a sprite painted with the `HostBackdropBrush` through Direct2D effects: saturation, Gaussian blur, and a flood color composited over them as the tint. Links that would do nothing are left out of the chain, and the compiled effects are reused.
4. The sprite is cut with a `CompositionGeometricClip`: a rounded rectangle, or for squircle corners a Direct2D path handed over as a `CompositionPath`.
5. The style's amounts live in one `CompositionPropertySet`, and the brush and the clips follow it through expressions. An animation runs on those few values, so regions added or moved halfway through it follow without a jump.
6. The effects are small hand-written `IGraphicsEffect` objects describing Direct2D effects, because Win2D does not run in desktop apps.
7. The window is subclassed with `SetWindowSubclass` to catch size, DPI and visibility changes, so Haze chains with other subclasses in any order. comctl32 5.82, which programs without a manifest load, exports it by ordinal only, so Haze looks it up by ordinal.
8. When DWM restarts (after a driver update or crash), windows lose what DWM knew about them. Haze sets the blur up again, and puts back the transparency that winit, tao and Tauri give their windows with `DwmEnableBlurBehindWindow`, so the window does not turn black.

## Requirements

- Windows 10 1809 or newer. `Haze::supported()` tells you whether this Windows has what the blur needs.
- `SetWindowCompositionAttribute` is undocumented, so a Windows update can break it.

`cargo test --features ffi` runs the blur on real windows, placed off screen, so it needs a desktop session.

## Related

- [window-vibrancy](https://github.com/tauri-apps/window-vibrancy) gives Tauri windows blur, acrylic and mica, rectangular.
- [Win32Acrylic](https://github.com/ALTaleX531/Win32Acrylic) draws acrylic through Composition in C++, over the whole window and without a clip.
- A [Microsoft Q&A thread](https://learn.microsoft.com/en-us/answers/a/152907) on blurring behind a Win32 or WPF window with the Visual Layer.

## License

MIT
