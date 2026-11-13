# tauri-plugin-haze

Rounded backdrop blur for Tauri 2 windows on Windows 10, from [Haze](https://github.com/spectrvm1337/Haze-blur). On macOS and Linux the plugin loads and does nothing, so it can be registered unconditionally.

Register the plugin, add `"haze:default"` to the window's capability, and make the window and the page background transparent.

```rust
tauri::Builder::default().plugin(tauri_plugin_haze::init())
```

From the page, with the `tauri-plugin-haze-api` package in `guest-js`:

```js
import * as haze from 'tauri-plugin-haze-api';

if (await haze.supported()) {
  await haze.attach({ preset: 'dark', radius: 20, corner: 'squircle' });
  await haze.animateTo({ preset: 'frost', radius: 32 }, 300);
  await haze.setRegions([{ x: 0, y: 0, width: 240, height: 700, radius: 16 }]);
}
```

Or with `invoke` directly: the commands are `attach`, `set_style`, `animate_to`, `fade`, `set_regions`, `cover_window`, `set_scale`, `detach` and `supported`, called as `plugin:haze|<command>`.

A style has an optional `preset` (`plain`, `dark`, `light`, `frost`) and any of `radius`, `strength`, `tint`, `tintColor`, `saturation` and `corner` (`'round'`, `'squircle'` or a CSS `superellipse()` number). `animate_to` and `fade` take `ms` and optionally `delay` (ms) and `easing` (`'linear'` or four `cubic-bezier` numbers).

MIT
