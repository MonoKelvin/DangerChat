import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, Square, X } from 'lucide-react';
import { IconButton } from './IconButton';

const win = getCurrentWindow();

/** 精简窗口控制（主窗口专用；alert 窗口无控制条）。 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void win
      .isMaximized()
      .then(setMaximized)
      .catch(() => {});
    void win
      .onResized(async () => setMaximized(await win.isMaximized().catch(() => false)))
      .then((u) => (unlisten = u))
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  return (
    <div className="flex h-full items-center">
      <IconButton
        size="sm"
        variant="ghost"
        title="最小化"
        onClick={() => void win.minimize()}
      >
        <Minus className="size-[18px]" strokeWidth={1.5} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        title={maximized ? '还原' : '最大化'}
        onClick={() => void win.toggleMaximize()}
      >
        <Square className="size-[15px]" strokeWidth={1.5} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        title="隐藏到托盘"
        className="hover:bg-[var(--danger)] hover:text-white"
        onClick={() => void win.hide()}
      >
        <X className="size-[18px]" strokeWidth={1.5} />
      </IconButton>
    </div>
  );
}
