import type { BrowserWindow } from 'electron';

/** `'round'`, `'squircle'`, or the `k` of CSS `superellipse(k)`: 1 round, 2 squircle. */
export type Corner = 'round' | 'squircle' | number;

export interface Style {
  preset?: 'plain' | 'dark' | 'light' | 'frost';
  /** Corner radius, CSS px. */
  radius?: number;
  /** Extra blur, CSS px. */
  strength?: number;
  /** How much of `tintColor` lies over the blur, 0 to 1. */
  tint?: number;
  tintColor?: [number, number, number];
  /** 1 leaves the colours alone. */
  saturation?: number;
  corner?: Corner;
}

/** A part of the window, in CSS px from its top-left corner. */
export interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
  /** Its own radius instead of the style's. */
  radius?: number;
  /** 0 to 1. */
  opacity?: number;
  corner?: Corner;
}

export class Haze {
  setStyle(style: Style): void;
  animateTo(style: Style, ms: number): void;
  fade(visible: boolean, ms?: number): void;
  setRegions(regions: Region[]): void;
  coverWindow(): void;
  setScale(scale: number | null): void;
  isAttached(): boolean;
  detach(): void;
}

export function supported(): boolean;
export function attach(window: BrowserWindow, style?: Style): Haze;
