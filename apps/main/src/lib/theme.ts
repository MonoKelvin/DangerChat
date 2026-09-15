/** 主题管理：dark / light / system 三态 + 主题色 accent（data-accent 属性驱动）。 */

export type Theme = 'dark' | 'light' | 'system';

const THEME_KEY = 'main-ui:theme';
const ACCENT_KEY = 'main-ui:accent';
const ORDER: Theme[] = ['dark', 'light', 'system'];

export interface AccentDef {
  id: string;
  name: string;
  /** 浅色主题下的品牌色 */
  light: string;
  /** 深色主题下的品牌色 */
  dark: string;
  /** 色相（度）：logo 变色与托盘同步用 */
  hue: number;
}

/** 主题色板（参考 monokelvin.studio），默认朱砂 */
export const ACCENTS: AccentDef[] = [
  { id: 'vermillion', name: '朱砂', light: '#c64b36', dark: '#ef7965', hue: 11 },
  { id: 'cobalt', name: '钴蓝', light: '#3559d8', dark: '#6f8cff', hue: 226 },
  { id: 'azure', name: '湛蓝', light: '#1677b8', dark: '#68b5e6', hue: 202 },
  { id: 'teal', name: '青碧', light: '#237b78', dark: '#6eb9b4', hue: 178 },
  { id: 'moss', name: '苔绿', light: '#527a59', dark: '#83b78b', hue: 132 },
  { id: 'amber', name: '琥珀', light: '#a86616', dark: '#d7a34e', hue: 34 },
  { id: 'violet', name: '紫晶', light: '#7554b8', dark: '#aa8be6', hue: 268 },
  { id: 'rose', name: '绯红', light: '#b74f73', dark: '#df84a4', hue: 339 },
];

export const THEME_LABEL: Record<Theme, string> = {
  dark: '深色',
  light: '浅色',
  system: '跟随系统',
};

export function getTheme(): Theme {
  const v = localStorage.getItem(THEME_KEY);
  return ORDER.includes(v as Theme) ? (v as Theme) : 'dark';
}

export function getAccent(): AccentDef {
  const v = localStorage.getItem(ACCENT_KEY);
  return ACCENTS.find((a) => a.id === v) ?? ACCENTS[0];
}

/** 解析 system → 实际生效的深/浅 */
function effective(t: Theme): 'dark' | 'light' {
  if (t !== 'system') return t;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/** 应用主题到 <html>（dark 类 + data-accent 属性）；system 模式下由调用方监听变化后重复调用 */
export function applyTheme(t: Theme): void {
  document.documentElement.classList.toggle('dark', effective(t) === 'dark');
}

export function applyAccent(a: AccentDef): void {
  document.documentElement.setAttribute('data-accent', a.id);
}

export function setTheme(t: Theme): void {
  localStorage.setItem(THEME_KEY, t);
  applyTheme(t);
}

export function setAccent(a: AccentDef): void {
  localStorage.setItem(ACCENT_KEY, a.id);
  applyAccent(a);
}

/** 点击循环：深色 → 浅色 → 系统 → 深色 */
export function nextTheme(t: Theme): Theme {
  return ORDER[(ORDER.indexOf(t) + 1) % ORDER.length];
}

/** 是否当前系统偏好为深色（system 模式下监听变化用） */
export function onSystemChange(cb: () => void): () => void {
  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  mq.addEventListener('change', cb);
  return () => mq.removeEventListener('change', cb);
}
