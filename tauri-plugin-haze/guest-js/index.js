// Haze for the page of a Tauri 2 app. Every call acts on the window the page is in.
import { invoke } from '@tauri-apps/api/core';

const call = (command, args) => invoke(`plugin:haze|${command}`, args);

/** Puts the blur under this window. Calling it again, after a reload for example, starts it over. */
export const attach = (style) => call('attach', { style });
/** Moves to `style` at once. */
export const setStyle = (style) => call('set_style', { style });
/** Moves to `style` over `ms`; the compositor runs the animation. */
export const animateTo = (style, ms, { delay, easing } = {}) => call('animate_to', { style, ms, delay, easing });
/** Fades the blur in or out, to go along with the page's own fade. */
export const fade = (visible, ms = 0, { delay, easing } = {}) => call('fade', { visible, ms, delay, easing });
/** Blurs only these parts of the window, in CSS px. An empty list blurs nothing. */
export const setRegions = (regions) => call('set_regions', { regions });
/** Blurs the whole window again. */
export const coverWindow = () => call('cover_window');
/** Physical px per CSS px; `null` follows the window's DPI. Pass `devicePixelRatio` if the page is zoomed. */
export const setScale = (scale) => call('set_scale', { scale: scale ?? null });
/** Takes the blur off the window. */
export const detach = () => call('detach');
/** Whether this system can show the blur (Windows 10 1809 or newer). */
export const supported = () => call('supported');
