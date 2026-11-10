# haze-blur

Rounded backdrop blur for Electron windows on Windows 10, from [Haze](https://github.com/spectrvm1337/Haze-blur). Windows x64 only.

`npm run build` builds `haze.dll`, which needs Rust. Call the package from the main process, on a window created with `transparent: true`.

```js
const haze = require('haze-blur');

const blur = haze.attach(win, { preset: 'dark', radius: 20 });
blur.animateTo({ preset: 'frost', radius: 32 }, 300);
blur.setRegions([{ x: 0, y: 0, width: 240, height: 700, radius: 16 }]);
blur.fade(false, 200);
blur.detach();
```

MIT
