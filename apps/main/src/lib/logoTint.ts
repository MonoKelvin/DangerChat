import { useEffect, useState } from 'react';

/** logo 主题色着色：真 HSL 色相旋转（只转色相，保留饱和度/亮度），与托盘 Rust 端 tint 同算法。
 *  CSS filter: hue-rotate() 走的是 YIQ 矩阵旋转，会顺带抬高饱和度——换主题色后 logo 偏艳，
 *  这里改用逐像素真 HSL 旋转，保证与主题色饱和度一致。 */

/** logo 基础色相（暖红 ≈11°）；与托盘 Rust 端 LOGO_HUE 同源 */
const LOGO_HUE = 11;
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

/** 就地色相旋转（灰色像素不动，alpha 保留），与托盘 tint(Active) 一致：只转色相。 */
function tintHue(data: Uint8ClampedArray, hueShift: number): void {
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 8) continue;
    const [h, s, l] = rgbToHsl(data[i] / 255, data[i + 1] / 255, data[i + 2] / 255);
    if (s < 1e-4) continue; // 无彩色像素不着色
    const [r, g, b] = hslToRgb((h + hueShift) % 360 < 0 ? (h + hueShift) % 360 + 360 : (h + hueShift) % 360, s, l);
    data[i] = Math.round(r * 255);
    data[i + 1] = Math.round(g * 255);
    data[i + 2] = Math.round(b * 255);
  }
}

let basePromise: Promise<ImageData | null> | null = null;
const cache = new Map<number, string>();

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

/** React hook：按主题色相返回着色后的 logo data URL（首帧 fallback 原图，加载/着色后替换）。 */
export function useTintedLogo(accentHue: number): string {
  const shift = Math.round(accentHue - LOGO_HUE);
  const [url, setUrl] = useState<string>(() => cache.get(shift) ?? LOGO_SRC);

  useEffect(() => {
    const cached = cache.get(shift);
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
      tintHue(out.data, shift);
      ctx.putImageData(out, 0, 0);
      const dataUrl = c.toDataURL('image/png');
      cache.set(shift, dataUrl);
      if (alive) setUrl(dataUrl);
    });
    return () => {
      alive = false;
    };
  }, [shift]);

  return url;
}
