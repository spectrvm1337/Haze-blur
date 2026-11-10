# Haze

A rounded-corner backdrop blur for Windows 10 windows, in Rust.

Windows 10 can only blur a whole rectangle behind a window (the blur and acrylic accents). Put a
rounded card in a transparent window and the blur sticks out of the corners as a square. Haze draws
the blur itself with `Windows.UI.Composition`, so it takes the card's shape.

It works with anything that gives you an `HWND`: Tauri, winit, a bare Win32 window.

## Use

```toml
[dependencies]
haze = "0.1"
```

```rust
use std::time::Duration;
use haze::{Haze, Style};

// On the thread that owns the window. `hwnd` is the raw HWND.
let haze = Haze::new(hwnd, Style { radius: 20.0, strength: 10.0 })?;

haze.resize(width_px, height_px, dpi_scale)?;       // call on every resize or scale change
haze.fade(false, Duration::from_millis(200))?;      // fade the blur with your own fade
```

Dropping the `Haze` removes the blur. The window itself has to be transparent (a transparent Tauri
window, or a Win32 window with `WS_EX_NOREDIRECTIONBITMAP`) and draw its content over the blur.

In Tauri, keep it in a `thread_local!` and create it inside `run_on_main_thread`, since it belongs to
the window's thread:

```rust
app.run_on_main_thread(move || {
    let hwnd = window.hwnd().unwrap().0;
    HAZE.with(|h| *h.borrow_mut() = Haze::new(hwnd, Style::default()).ok());
})?;
```

Try it without any framework:

```
cargo run --example demo
```

Space fades the blur out and in, Esc closes, and the window drags by its body.

## How it works

1. `SetWindowCompositionAttribute` with accent state 5 (`ACCENT_ENABLE_HOSTBACKDROP`) lets a
   `HostBackdropBrush` see what is behind the window. The call is undocumented.
2. A `Compositor` gets a `DesktopWindowTarget` on the window (not topmost, so the page draws over it).
3. One sprite, painted with the `HostBackdropBrush` through a Gaussian blur effect, is clipped with a
   `CompositionGeometricClip` over a rounded rectangle. The clip is what gives the blur its shape.
4. The blur effect is a small hand-written `IGraphicsEffect` that describes `CLSID_D2D1GaussianBlur`.
   Win2D normally does this, but it does not run in desktop apps.

## Status

- Tested on Windows 10 22H2 (build 19045). Not tested on other builds or on Windows 11.
- `HostBackdropBrush` needs Windows 10 1809 or newer.
- Acrylic and blur accents are known to make windows laggy while dragging on 1903 and newer. Haze
  has not been measured for that.
- Because it relies on an undocumented call, a Windows update can break it.

## Related

- [window-vibrancy](https://github.com/tauri-apps/window-vibrancy): blur, acrylic and mica for Tauri,
  rectangular.
- [Win32Acrylic](https://github.com/ALTaleX531/Win32Acrylic): acrylic through Composition in C++, a
  full-window rectangle without a clip.
- [A Microsoft Q&A thread](https://learn.microsoft.com/en-us/answers/a/152907) on blurring behind a
  Win32 or WPF window with the Visual Layer.

## License

MIT
