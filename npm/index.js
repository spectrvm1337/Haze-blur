// Haze for Electron. Call it from the main process, which is the thread that owns the window.
const path = require('path');
const koffi = require('koffi');

const lib = koffi.load(path.join(__dirname, 'bin', 'x64', 'haze.dll'));

// Handles are declared as plain integers: a window's handle comes as a number, not as memory to read.
const native = {
  supported: lib.func('int haze_supported()'),
  attach: lib.func('uintptr_t haze_attach(uintptr_t hwnd, float radius, float strength, float tint, uint32_t rgb, float saturation)'),
  setStyle: lib.func('void haze_set_style(uintptr_t haze, float radius, float strength, float tint, uint32_t rgb, float saturation, uint32_t millis)'),
  fade: lib.func('void haze_fade(uintptr_t haze, int visible, uint32_t millis)'),
  setRegions: lib.func('void haze_set_regions(uintptr_t haze, float *rects, uint32_t count)'),
  detach: lib.func('void haze_detach(uintptr_t haze)'),
};

const PRESETS = {
  plain: { radius: 20, strength: 10, tint: 0, tintColor: [0, 0, 0], saturation: 1 },
  dark: { radius: 20, strength: 14, tint: 0.45, tintColor: [10, 10, 14], saturation: 1 },
  light: { radius: 20, strength: 14, tint: 0.4, tintColor: [244, 245, 250], saturation: 1 },
  frost: { radius: 20, strength: 24, tint: 0.16, tintColor: [255, 255, 255], saturation: 1.25 },
};

function resolve(style = {}) {
  const s = { ...PRESETS[style.preset || 'plain'], ...style };
  const rgb = ((s.tintColor[0] & 255) << 16) | ((s.tintColor[1] & 255) << 8) | (s.tintColor[2] & 255);
  return [s.radius, s.strength, s.tint, rgb >>> 0, s.saturation];
}

class Haze {
  constructor(handle) {
    this.handle = handle;
  }
  /** Moves to `style` at once. */
  setStyle(style) {
    this.animateTo(style, 0);
  }
  /** Moves to `style` over `ms`; the compositor runs the animation. */
  animateTo(style, ms) {
    if (this.handle) native.setStyle(this.handle, ...resolve(style), ms);
  }
  /** Fades the blur in or out with your own fade. */
  fade(visible, ms = 0) {
    if (this.handle) native.fade(this.handle, visible ? 1 : 0, ms);
  }
  /** Blurs only these parts of the window; an empty list goes back to the whole window. */
  setRegions(regions) {
    if (!this.handle) return;
    const rects = new Float32Array(regions.length * 5);
    regions.forEach((r, i) => rects.set([r.x, r.y, r.width, r.height, r.radius === undefined ? -1 : r.radius], i * 5));
    native.setRegions(this.handle, regions.length ? rects : null, regions.length);
  }
  /** Takes the blur off the window. The handle is finished after this. */
  detach() {
    if (this.handle) native.detach(this.handle);
    this.handle = 0;
  }
}

/** Whether this Windows has what the blur needs. */
function supported() {
  return native.supported() === 1;
}

/** Puts the blur under a BrowserWindow created with `transparent: true`. */
function attach(browserWindow, style) {
  const buffer = browserWindow.getNativeWindowHandle();
  const [radius, strength, tint, rgb, saturation] = resolve(style);
  const handle = native.attach(buffer.readBigUInt64LE(0), radius, strength, tint, rgb, saturation);
  if (!handle) throw new Error('haze: could not start the blur on this window');
  return new Haze(handle);
}

module.exports = { attach, supported, Haze };
