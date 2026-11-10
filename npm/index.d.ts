import type { BrowserWindow } from 'electron';

export interface Style {
  preset?: 'plain' | 'dark' | 'light' | 'frost';
  radius?: number;
  strength?: number;
  tint?: number;
  tintColor?: [number, number, number];
  saturation?: number;
}

export interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
  radius?: number;
}

export class Haze {
  setStyle(style: Style): void;
  animateTo(style: Style, ms: number): void;
  fade(visible: boolean, ms?: number): void;
  setRegions(regions: Region[]): void;
  detach(): void;
}

export function supported(): boolean;
export function attach(window: BrowserWindow, style?: Style): Haze;
