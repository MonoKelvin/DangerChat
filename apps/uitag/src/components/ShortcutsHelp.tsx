import { useEffect, useState } from 'react';
import { IconKeyboard } from '@tabler/icons-react';
import { Button } from './ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-border bg-foreground/5 px-1.5 py-0.5 font-mono text-[11px] leading-none text-muted-foreground">
      {children}
    </kbd>
  );
}

const GROUPS: { title: string; items: [React.ReactNode, string][] }[] = [
  {
    title: '绘制标注',
    items: [
      [<span key="1">拖拽</span>, '按当前标签绘制标注框'],
      [<Kbd key="2">1</Kbd>, '按数字快速切换标签（选中即进入绘制）'],
      [
        <>
          <Kbd>Esc</Kbd> / 点击空白
        </>,
        '取消绘制或退出绘制模式',
      ],
      [<span key="4">右键标注框</span>, '修改标签 / 删除'],
    ],
  },
  {
    title: '编辑标注',
    items: [
      [<span key="1">拖动框体</span>, '移动标注框'],
      [<span key="2">拖动锚点</span>, '调整大小'],
      [
        <>
          <Kbd>Del</Kbd> / <Kbd>Backspace</Kbd>
        </>,
        '删除选中框',
      ],
      [
        <>
          <Kbd>Ctrl+Z</Kbd> · <Kbd>Ctrl+Shift+Z</Kbd>
        </>,
        '撤销 / 重做',
      ],
    ],
  },
  {
    title: '导航与视图',
    items: [
      [
        <>
          <Kbd>↑</Kbd> · <Kbd>↓</Kbd>
        </>,
        '上一张 / 下一张图片',
      ],
      [
        <>
          <Kbd>Ctrl</Kbd> + 滚轮
        </>,
        '缩放画布',
      ],
    ],
  },
];

/** 快捷键帮助弹层：右上角触发；悬浮卡片带阴影与高斯模糊，背景不压暗。 */
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
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            className="size-8 text-muted-foreground hover:shadow-sm"
            onClick={() => setOpen((v) => !v)}
          >
            <IconKeyboard className="size-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">快捷键</TooltipContent>
      </Tooltip>

      {open && (
        <>
          {/* 透明点击层：点外部关闭，不遮挡背景 */}
          <div className="fixed inset-0 z-40" onPointerDown={close} />
          <div
            role="dialog"
            className="animate-in fade-in-0 zoom-in-95 slide-in-from-top-1 fixed top-14 right-3 z-50 w-96 rounded-xl border border-border/70 bg-popover/75 p-5 text-popover-foreground shadow-2xl backdrop-blur-2xl outline-none"
          >
            <div className="mb-4 flex items-center gap-2.5">
              <IconKeyboard className="size-4 text-primary" />
              <h2 className="text-sm font-semibold">快捷键</h2>
            </div>
            <div className="space-y-4">
              {GROUPS.map((g) => (
                <section key={g.title}>
                  <p className="mb-2 text-[11px] font-medium tracking-widest text-muted-foreground/80 uppercase">
                    {g.title}
                  </p>
                  <ul className="space-y-1.5">
                    {g.items.map(([keys, desc], i) => (
                      <li key={i} className="flex items-center justify-between gap-4 text-xs">
                        <span className="text-muted-foreground">{desc}</span>
                        <span className="flex shrink-0 items-center gap-1 text-foreground/90">{keys}</span>
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
