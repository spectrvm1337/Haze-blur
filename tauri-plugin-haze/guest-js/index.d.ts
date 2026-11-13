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

export interface Timing {
  /** Milliseconds to wait before starting. */
  delay?: number;
  /** `'linear'` or the four numbers of CSS `cubic-bezier()`. */
  easing?: 'linear' | [number, number, number, number];
}

export function attach(style?: Style): Promise<void>;
export function setStyle(style: Style): Promise<void>;
export function animateTo(style: Style, ms: number, timing?: Timing): Promise<void>;
export function fade(visible: boolean, ms?: number, timing?: Timing): Promise<void>;
export function setRegions(regions: Region[]): Promise<void>;
export function coverWindow(): Promise<void>;
export function setScale(scale: number | null): Promise<void>;
export function detach(): Promise<void>;
export function supported(): Promise<boolean>;
