# tauri-plugin-haze

Rounded backdrop blur for Tauri 2 windows on Windows 10, from [Haze](https://github.com/spectrvm1337/Haze-blur). Windows only.

Register the plugin, add `"haze:default"` to the window's capability, and make the window and the page background transparent.

```rust
tauri::Builder::default().plugin(tauri_plugin_haze::init())
```

```js
await invoke('plugin:haze|attach', { style: { preset: 'dark', radius: 20 } });
await invoke('plugin:haze|animate_to', { style: { preset: 'frost', radius: 32 }, ms: 300 });
```

The commands are `attach`, `set_style`, `animate_to`, `fade`, `set_regions` and `detach`. A style has an optional `preset` (`dark`, `light`, `frost`) and any of `radius`, `strength`, `tint`, `tintColor` and `saturation`.

MIT
