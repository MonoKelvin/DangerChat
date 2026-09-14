import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { IconCopy, IconMinus, IconSquare, IconX } from '@tabler/icons-react';
import { useStore } from '../store';
import { cn } from '@/lib/utils';

const win = getCurrentWindow();

/** Windows 11 风格窗口控制组（无衬线、hover 高亮、关闭红色）。
 *  关闭前先冲刷标注自动保存（beforeunload 在 WebView 关闭路径上不可靠）。 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    win
      .isMaximized()
      .then(setMaximized)
      .catch(() => {});
    win
      .onResized(async () => setMaximized(await win.isMaximized().catch(() => false)))
      .then((u) => (unlisten = u))
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  const btn =
    'flex h-full w-11 items-center justify-center text-muted-foreground transition-colors disabled:opacity-40';

  const close = async () => {
    try {
      await useStore.getState().flush();
    } catch {
      // 保存失败也要让用户能关窗（数据已在 800ms debounce 内大概率落盘）
    }
    await win.destroy();
  };

  return (
    <div className="flex h-full">
      <button
        className={cn(btn, 'hover:bg-foreground/10 hover:text-foreground')}
        onClick={() => void win.minimize()}
        data-tip="最小化"
      >
        <IconMinus className="size-3.5" strokeWidth={1.75} />
      </button>
      <button
        className={cn(btn, 'hover:bg-foreground/10 hover:text-foreground')}
        onClick={() => void win.toggleMaximize()}
        data-tip={maximized ? '还原' : '最大化'}
      >
        {maximized ? (
          <IconCopy className="size-3" strokeWidth={1.75} />
        ) : (
          <IconSquare className="size-3" strokeWidth={1.75} />
        )}
      </button>
      <button
        className={cn(btn, 'hover:bg-[#c42b1c] hover:text-white')}
        onClick={() => void close()}
        data-tip="关闭"
      >
        <IconX className="size-4" strokeWidth={1.75} />
      </button>
    </div>
  );
}
