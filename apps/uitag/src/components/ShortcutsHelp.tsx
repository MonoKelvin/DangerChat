import { useEffect, useState } from 'react';
import {
  IconKeyboard,
  IconMouse,
  IconPencil,
  IconStack2,
} from '@tabler/icons-react';
import { Button } from './ui/button';
import { SHORTCUTS, SHORTCUT_GROUPS, type GroupId } from '../lib/shortcuts';

/** 分组 → 类型图标 */
const GROUP_ICONS: Record<GroupId, React.ReactNode> = {
  draw: <IconPencil className="size-3.5" />,
  edit: <IconStack2 className="size-3.5" />,
  nav: <IconMouse className="size-3.5" />,
};

/** 快捷键帮助弹层：右上角触发；窗口居中悬浮卡片，大重阴影 + 高斯模糊，背景不压暗。
 *  内容直接渲染 lib/shortcuts 注册表，与按键分发共用同一份定义。 */
export function ShortcutsHelp() {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open]);

  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        className="size-8 text-muted-foreground hover:shadow-sm"
        onClick={() => setOpen((v) => !v)}
        data-tip="快捷键"
      >
        <IconKeyboard className="size-4" />
      </Button>

      {open && (
        <>
          {/* 透明点击层：点外部关闭，不遮挡背景 */}
          <div className="fixed inset-0 z-40" onPointerDown={close} />
          <div
            role="dialog"
            className="animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-4 fixed top-1/2 left-1/2 z-50 w-96 -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border/70 bg-popover/75 p-5 text-popover-foreground shadow-2xl shadow-black/60 backdrop-blur-2xl outline-none"
          >
            <div className="mb-4 flex items-center gap-2.5">
              <IconKeyboard className="size-4 text-primary" />
              <h2 className="text-sm font-semibold">快捷键</h2>
            </div>
            <div className="space-y-5">
              {SHORTCUT_GROUPS.map((g) => (
                <section key={g.id}>
                  <p className="mb-2.5 flex items-center gap-1.5 text-[13px] font-semibold tracking-wide text-foreground/80">
                    <span className="text-primary">{GROUP_ICONS[g.id]}</span>
                    {g.title}
                  </p>
                  <ul className="space-y-1.5">
                    {SHORTCUTS.filter((s) => s.group === g.id).map((s) => (
                      <li key={s.id} className="flex items-center justify-between gap-4 text-xs">
                        <span className="text-muted-foreground">{s.desc}</span>
                        <span className="flex shrink-0 items-center gap-1 text-foreground/90">{s.keys}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          </div>
        </>
      )}
    </>
  );
}
