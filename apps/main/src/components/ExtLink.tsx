import { type ReactNode } from 'react';
import * as api from '../lib/commands';

/**
 * 外部链接（主题色文字，点击经后端在外部浏览器打开）。
 *
 * **为什么不用 `<a href target="_blank">`**：Tauri 的 WebView 不会把新窗口请求交给
 * 系统浏览器——没有注册 new-window 处理器时，点击表现为「毫无反应」。
 * 必须显式调后端 `open_external`（内部走 `tauri-plugin-opener`），
 * 且后端只放行 `https://`（见 `src-tauri/src/lib.rs::open_external`）。
 *
 * 渲染成 `<button>` 是有意的：它没有 href，不会被 WebView 当导航处理。
 * 调用方把它内联放进 `<p>` 即可（button 属 phrasing content）。
 */
export function ExtLink({ url, children }: { url: string; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={() => void api.openExternal(url).catch(() => {})}
      className="font-medium text-[var(--brand)] underline underline-offset-2 transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
    >
      {children}
    </button>
  );
}
