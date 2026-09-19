import { useEffect, useState } from 'react';

/** logo 主题色着色：彩色像素**直接采用主题色的 H+S**，只保留像素自身亮度 L 维持明暗层次。
 *  与托盘 Rust 端 tint(Active) 同算法。
 *
 *  为何不是「旋转色相、保留原饱和度」：旧做法保留 logo 原图饱和度（≈0.66），换到低饱和主题色
 *  （苔绿/青碧）时明显偏艳、与色板对不上。改为固定用主题色饱和度，颜色即主题色本身，零偏差。
 *  CSS hue-rotate 更不可用——它走 YIQ 矩阵近似，还会抬饱和度。 */

const LOGO_SRC = 'app-icon.png';

function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d < 1e-6) return [0, 0, l];
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === r) h = (((g - b) / d) % 6 + 6) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

function hueToRgb(p: number, q: number, t: number): number {
  if (t < 0) t += 1;
  if (t > 1) t -= 1;
  if (t < 1 / 6) return p + (q - p) * 6 * t;
  if (t < 0.5) return q;
  if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
  return p;
}

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  if (s < 1e-6) return [l, l, l];
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const t = h / 360;
  return [hueToRgb(p, q, t + 1 / 3), hueToRgb(p, q, t), hueToRgb(p, q, t - 1 / 3)];
}

/** 就地着色（灰色像素不动，alpha 保留）：彩色像素 H/S 换成主题色，只保留自身 L。
 *  与托盘 tint(Active) 一致。hue 度、sat 0-1。 */
function tintToAccent(data: Uint8ClampedArray, hue: number, sat: number): void {
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 8) continue;
    const [, s, l] = rgbToHsl(data[i] / 255, data[i + 1] / 255, data[i + 2] / 255);
    if (s < 1e-4) continue; // 无彩色像素（黑白灰描边）不着色，保持中性
    const [r, g, b] = hslToRgb(hue, sat, l);
    data[i] = Math.round(r * 255);
    data[i + 1] = Math.round(g * 255);
    data[i + 2] = Math.round(b * 255);
  }
}

let basePromise: Promise<ImageData | null> | null = null;
/** 缓存键 = `${hue}:${sat}`（着色只由主题色决定）。 */
const cache = new Map<string, string>();

function loadBase(): Promise<ImageData | null> {
  if (basePromise) return basePromise;
  basePromise = new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement('canvas');
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext('2d');
      if (!ctx) return resolve(null);
      ctx.drawImage(img, 0, 0);
      try {
        resolve(ctx.getImageData(0, 0, c.width, c.height));
      } catch {
        resolve(null);
      }
    };
    img.onerror = () => resolve(null);
    img.src = LOGO_SRC;
  });
  return basePromise;
}

/** React hook：按主题色（hue 度 + sat 0-100）返回着色后的 logo data URL
 *  （首帧 fallback 原图，加载/着色后替换）。 */
export function useTintedLogo(accentHue: number, accentSat: number): string {
  const key = `${Math.round(accentHue)}:${Math.round(accentSat)}`;
  const [url, setUrl] = useState<string>(() => cache.get(key) ?? LOGO_SRC);

  useEffect(() => {
    const cached = cache.get(key);
    if (cached) {
      setUrl(cached);
      return;
    }
    let alive = true;
    void loadBase().then((base) => {
      if (!alive || !base) return;
      const c = document.createElement('canvas');
      c.width = base.width;
      c.height = base.height;
      const ctx = c.getContext('2d');
      if (!ctx) return;
      const out = new ImageData(new Uint8ClampedArray(base.data), base.width, base.height);
      tintToAccent(out.data, accentHue, accentSat / 100);
      ctx.putImageData(out, 0, 0);
      const dataUrl = c.toDataURL('image/png');
      cache.set(key, dataUrl);
      if (alive) setUrl(dataUrl);
    });
    return () => {
      alive = false;
    };
  }, [key, accentHue, accentSat]);

  return url;
}
