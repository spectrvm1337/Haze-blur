# Changelog

## 0.4.0

Added
- Squircle corners: `Corner::Squircle` and any CSS `superellipse(k)` as `Corner::Superellipse(k)`, on the style or per region.
- `Region::opacity`, so the blur of one card can fade with it.
- `Transition`: `animate_to` and `fade` take a delay and a curve as well as a duration. A plain `Duration` still works.
- `Haze::set_scale`, for physical pixels or a zoomed webview.
- `Haze::cover_window`, `Haze::is_attached` and `Haze::last_error`.
- The blur and the window's transparency come back after DWM restarts. Before, a Tauri or winit window could turn black there.
- C interface: `haze_set_regions_ex`, `haze_set_corner`, `haze_set_scale`, `haze_is_attached`, `haze_last_error`.
- Tauri plugin: `cover_window`, `set_scale` and `supported`, `delay` and `easing` on `animate_to` and `fade`, and a JS package in `tauri-plugin-haze/guest-js`.

Changed
- `set_regions(&[])` blurs nothing. `cover_window()` goes back to the whole window. In C, a `rects` of NULL still means the whole window.
- `strength` is in CSS px and scaled by the window's DPI, like the radius.
- The window is subclassed with `SetWindowSubclass`, looked up by ordinal so it works without a comctl32 manifest. It chains with other subclasses in any order.
- Regions are updated in place, and unchanged ones cost nothing, so sending them every frame is cheap.
- A region added or moved during an animation follows it instead of jumping to the end.
- The tint is part of the effect, so each region is one sprite instead of two. Effects at their neutral value are left out, and compiled effects and the compositor are shared by the windows of a thread.
- Calls from other threads are merged: a burst of them costs one round on the window's thread.
- Haze uses a registered window message instead of one from the program's `WM_APP` range.
- On detach the window gets back the accent it had before.
- `Haze::supported()` also checks for `CompositionGeometricClip`, so it answers false on Windows 10 1803.
- The Tauri plugin builds on macOS and Linux and does nothing there, and forgets the blur of a closed window.
- The npm package finds `haze.dll` in `app.asar.unpacked` in packaged apps, reports why `attach` failed, and accepts `corner`, `opacity`, `coverWindow`, `setScale` and `isAttached`.

Fixed
- Attaching again after `detach` failed for good when other code had subclassed the window after Haze.
- Handles kept queueing calls after a detach or once the window was gone.
- `set_regions` and resizing during an animation snapped the radius and the tint to their end values.
- A poisoned lock or a NaN in a style or region could crash or break the blur.

## 0.3.0

Added
- `Haze::set_regions` and `Region`, to blur parts of a window instead of all of it.
- `Style::saturation`, animated together with the rest of the style.
- `tauri-plugin-haze`, which drives the blur from JavaScript, and `examples/tauri`.
- The `haze-blur` npm package for Electron, in `npm`.
- `examples/winit.rs` and tests that run the blur on real windows.
- `haze_set_regions` in the C interface.

Changed
- The blur stops drawing while the window is hidden or minimized.
- `haze_attach` and `haze_set_style` take a `saturation` argument.
- Haze also sets `DWMWA_USE_HOSTBACKDROPBRUSH`, the documented switch on Windows 11. This has not been tested on Windows 11.

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
