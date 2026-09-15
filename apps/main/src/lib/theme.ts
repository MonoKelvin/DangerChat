/** 主题管理：dark / light / system 三态，html.dark 一键切换（CSS 变量驱动）。 */

export type Theme = 'dark' | 'light' | 'system';

const KEY = 'dangerchat:theme';
const ORDER: Theme[] = ['dark', 'light', 'system'];

export const THEME_LABEL: Record<Theme, string> = {
  dark: '深色',
  light: '浅色',
  system: '跟随系统',
};

export function getTheme(): Theme {
  const v = localStorage.getItem(KEY);
  return ORDER.includes(v as Theme) ? (v as Theme) : 'dark';
}

/** 解析 system → 实际生效的深/浅 */
function effective(t: Theme): 'dark' | 'light' {
  if (t !== 'system') return t;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/** 应用主题到 <html>；system 模式下由调用方监听系统变化后重复调用 */
export function applyTheme(t: Theme): void {
  document.documentElement.classList.toggle('dark', effective(t) === 'dark');
}

export function setTheme(t: Theme): void {
  localStorage.setItem(KEY, t);
  applyTheme(t);
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
