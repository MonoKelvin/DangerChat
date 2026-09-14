import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { cn } from '../lib/utils';

const win = getCurrentWindow();

/** Windows 11 风格窗口控制（主窗口专用；alert 窗口无控制条）。 */
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

  const btn =
    'flex h-full w-11 items-center justify-center text-muted-foreground transition-colors';

  return (
    <div className="flex h-full">
      <button
        className={cn(btn, 'hover:bg-foreground/10 hover:text-foreground')}
        title="最小化"
        onClick={() => void win.minimize()}
      >
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="1" y="5.5" width="10" height="1" fill="currentColor" />
        </svg>
      </button>
      <button
        className={cn(btn, 'hover:bg-foreground/10 hover:text-foreground')}
        title={maximized ? '还原' : '最大化'}
        onClick={() => void win.toggleMaximize()}
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1">
          <rect x="1.5" y="1.5" width="9" height="9" />
        </svg>
      </button>
      <button
        className={cn(btn, 'hover:bg-[#c42b1c] hover:text-white')}
        title="关闭（最小化到托盘）"
        onClick={() => void win.hide()}
      >
        <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" strokeWidth="1.1">
          <path d="M2 2 L10 10 M10 2 L2 10" />
        </svg>
      </button>
    </div>
  );
}
