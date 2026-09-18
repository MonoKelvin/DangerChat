import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, X } from 'lucide-react';

const win = getCurrentWindow();

/** macOS 式窗口控制：黄=最小化、红=关闭。
 *  从左到右，无绿点（maximizable: false）。图标仅在 hover 时浮现。
 *
 * `onClose` 决定红点的行为：
 * - 首启告知页（未同意）：直接退出程序（关闭=退出）——此时托盘尚未创建。
 * - 已同意后：隐藏到托盘（关闭=隐藏到托盘）。 */
export function WindowControls({ onClose }: { onClose: () => void }) {
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
        data-tip="关闭（退出软件）"
        onClick={() => void onClose()}
      >
        <X
          className="size-2.5 text-black/55 opacity-0 transition-opacity group-hover:opacity-100"
          strokeWidth={2.75}
        />
      </button>
    </div>
  );
}
