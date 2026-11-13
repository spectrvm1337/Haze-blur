// Haze for Electron. Attach from the main process, which is the thread that owns the window.
const path = require('path');
const koffi = require('koffi');

// Windows cannot load a library out of an asar archive: in a packaged app the DLL is read from
// app.asar.unpacked (see "asarUnpack" in the README).
const dll = path.join(__dirname, 'bin', 'x64', 'haze.dll').replace(/app\.asar([\\/])/, 'app.asar.unpacked$1');
const lib = koffi.load(dll);

// Handles are declared as plain integers: a window's handle comes as a number, not as memory to read.
const native = {
  supported: lib.func('int haze_supported()'),
  lastError: lib.func('int32_t haze_last_error()'),
  attach: lib.func('uintptr_t haze_attach(uintptr_t hwnd, float radius, float strength, float tint, uint32_t rgb, float saturation)'),
  setStyle: lib.func('void haze_set_style(uintptr_t haze, float radius, float strength, float tint, uint32_t rgb, float saturation, uint32_t millis)'),
  setCorner: lib.func('void haze_set_corner(uintptr_t haze, float k)'),
  fade: lib.func('void haze_fade(uintptr_t haze, int visible, uint32_t millis)'),
  setRegions: lib.func('void haze_set_regions_ex(uintptr_t haze, float *rects, uint32_t count)'),
  setScale: lib.func('void haze_set_scale(uintptr_t haze, float scale)'),
  isAttached: lib.func('int haze_is_attached(uintptr_t haze)'),
  detach: lib.func('void haze_detach(uintptr_t haze)'),
};

const PRESETS = {
  plain: { radius: 20, strength: 10, tint: 0, tintColor: [0, 0, 0], saturation: 1 },
  dark: { radius: 20, strength: 14, tint: 0.45, tintColor: [10, 10, 14], saturation: 1 },
  light: { radius: 20, strength: 14, tint: 0.4, tintColor: [244, 245, 250], saturation: 1 },
  frost: { radius: 20, strength: 24, tint: 0.16, tintColor: [255, 255, 255], saturation: 1.25 },
};

/** The `k` of CSS `superellipse(k)` for a corner given as 'round', 'squircle' or a number. */
function cornerK(corner) {
  if (corner === undefined || corner === null) return 0;
  if (corner === 'round') return 1;
  if (corner === 'squircle') return 2;
  if (typeof corner === 'number' && corner > 0) return corner;
  throw new TypeError(`haze: unknown corner ${JSON.stringify(corner)}`);
}

function resolve(style = {}) {
  const preset = PRESETS[style.preset || 'plain'];
  if (!preset) throw new TypeError(`haze: unknown preset ${JSON.stringify(style.preset)}`);
  const s = { ...preset };
  for (const [key, value] of Object.entries(style)) if (value !== undefined) s[key] = value;
  const [r, g, b] = s.tintColor;
  const rgb = ((r & 255) << 16) | ((g & 255) << 8) | (b & 255);
  return { args: [s.radius, s.strength, s.tint, rgb >>> 0, s.saturation], corner: s.corner };
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
    if (!this.handle) return;
    const { args, corner } = resolve(style);
    // The corner shape changes at once (the C side leaves an unchanged one alone); set it first so
    // the radius animates on the new shape.
    if (corner !== undefined) native.setCorner(this.handle, cornerK(corner) || 1);
    native.setStyle(this.handle, ...args, ms);
  }
  /** Fades the blur in or out with your own fade. */
  fade(visible, ms = 0) {
    if (this.handle) native.fade(this.handle, visible ? 1 : 0, ms);
  }
  /** Blurs only these parts of the window, in CSS px. An empty list blurs nothing. */
  setRegions(regions) {
    if (!this.handle) return;
    // Never a null pointer here: null means the whole window.
    const rects = new Float32Array(Math.max(regions.length, 1) * 7);
    regions.forEach((r, i) => {
      const radius = r.radius === undefined ? -1 : r.radius;
      const opacity = r.opacity === undefined ? 1 : r.opacity;
      rects.set([r.x, r.y, r.width, r.height, radius, opacity, cornerK(r.corner)], i * 7);
    });
    native.setRegions(this.handle, rects, regions.length);
  }
  /** Blurs the whole window again. */
  coverWindow() {
    if (this.handle) native.setRegions(this.handle, null, 0);
  }
  /** Physical px per unit; `null` follows the window's DPI. */
  setScale(scale) {
    if (this.handle) native.setScale(this.handle, scale > 0 ? scale : 0);
  }
  /** Whether the blur is still on the window: false after detach() or once the window is closed. */
  isAttached() {
    return Boolean(this.handle) && native.isAttached(this.handle) === 1;
  }
  /** Takes the blur off the window. The handle is finished after this. */
  detach() {
    if (this.handle) native.detach(this.handle);
    this.handle = 0;
  }
}

/** Whether this Windows has what the blur needs (Windows 10 1809 or newer). */
function supported() {
  return native.supported() === 1;
}

/** Puts the blur under a BrowserWindow created with `transparent: true`. */
function attach(browserWindow, style) {
  const buffer = browserWindow.getNativeWindowHandle();
  const { args, corner } = resolve(style);
  const handle = native.attach(buffer.readBigUInt64LE(0), ...args);
  if (!handle) {
    const code = (native.lastError() >>> 0).toString(16).toUpperCase();
    throw new Error(`haze: could not start the blur on this window (HRESULT 0x${code})`);
  }
  const haze = new Haze(handle);
  if (corner !== undefined) native.setCorner(handle, cornerK(corner) || 1);
  // The blur goes with the window; the handle should not outlive it.
  browserWindow.once('closed', () => haze.detach());
  return haze;
}

module.exports = { attach, supported, Haze };
