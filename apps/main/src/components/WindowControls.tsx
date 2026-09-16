import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, X } from 'lucide-react';

const win = getCurrentWindow();

/** macOS 红绿灯式窗口控制（主窗口专用；alert 窗口无控制条）。
 *  从左到右：黄=最小化、红=关闭（隐藏到托盘）；无绿点（maximizable: false）。
 *  图标仅在 hover 时浮现。 */
export function WindowControls() {
  return (
    <div className="flex items-center gap-3">
      <button
        className="group flex size-3 items-center justify-center rounded-full border border-[#d89e24] bg-[#febc2e] transition-colors"
        data-tip="最小化"
        onClick={() => void win.minimize()}
      >
        <Minus
          className="size-2.5 text-black/55 opacity-0 transition-opacity group-hover:opacity-100"
          strokeWidth={3}
        />
      </button>
      <button
        className="group flex size-3 items-center justify-center rounded-full border border-[#e0443e] bg-[#ff5f57] transition-colors"
        data-tip="关闭（隐藏到托盘）"
        onClick={() => void win.hide()}
      >
        <X
          className="size-2.5 text-black/55 opacity-0 transition-opacity group-hover:opacity-100"
          strokeWidth={2.75}
        />
      </button>
    </div>
  );
}
