# haze-blur

Rounded backdrop blur for Electron windows on Windows 10, from [Haze](https://github.com/spectrvm1337/Haze-blur). Windows x64 only.

`npm run build` builds `haze.dll`, which needs Rust. Call the package from the main process, on a window created with `transparent: true`.

```js
const haze = require('haze-blur');

const blur = haze.attach(win, { preset: 'dark', radius: 20, corner: 'squircle' });
blur.animateTo({ preset: 'frost', radius: 32 }, 300);
blur.setRegions([{ x: 0, y: 0, width: 240, height: 700, radius: 16, opacity: 0.8 }]);
blur.coverWindow(); // back to the whole window
blur.fade(false, 200);
blur.detach();
```

`setRegions([])` blurs nothing. Regions, radii and strength are in CSS px; `setScale(1)` switches to physical pixels.

## Packaged apps

Windows cannot load a DLL from inside `app.asar`. Unpack it, and the package finds it in `app.asar.unpacked`. With electron-builder:

```json
"asarUnpack": ["node_modules/haze-blur/bin/**"]
```

MIT
