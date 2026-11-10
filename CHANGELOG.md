# Changelog

## 0.2.0

Added
- `Haze::attach(&window, style)` for anything with a window handle, and `Haze::attach_hwnd` for a raw `HWND`. The blur follows the window's size and DPI by itself.
- `animate_to(style, duration)`. The radius, the strength, the tint and its color move together, and the compositor runs the animation.
- `Style::tint_color`, the presets `Style::dark()`, `light()` and `frost()`, and builder methods `radius`, `strength`, `tint` and `tint_color`.
- `Haze::supported()` checks that this Windows has `HostBackdropBrush`.
- A C interface in `haze.dll`, built with `--features ffi`.
- `CHANGELOG.md` and an animated demo in the README.

Changed
- `Haze` is a handle that can be cloned and sent to other threads. Calls are carried out on the window's thread.
- Dropping a `Haze` no longer removes the blur. It goes away with the window, or on `detach()`.
- `set_style` and `fade` take `&self` and return nothing.
- `Style` is `#[non_exhaustive]`, so build it from a preset instead of a struct literal.
- The blur strength is a named effect property. Changing it no longer builds the effect again.
- The example changes its style on its own and no longer calls anything per frame.
- The demo window is topmost.

Removed
- `Haze::new`, `resize` and `fade_with`. Size and DPI are tracked after `attach`.

## 0.1.0

First version: one rounded blur under a window, with `resize`, `fade` and a `Style` of radius, strength and tint.
