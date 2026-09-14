import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { IconLock, IconLockOpen, IconTrash } from '@tabler/icons-react';
import { useStore } from '../store';

interface MenuState {
  x: number;
  y: number;
  index: number;
  locked: boolean;
}

/** 右键改标签浮层：AnnoCanvas 派发 uitag-relabel（带真实鼠标坐标 + 锁定态）。 */
export function RelabelMenu() {
  const tags = useStore((s) => s.tags.tags);
  const relabelBox = useStore((s) => s.relabelBox);
  const deleteSelected = useStore((s) => s.deleteSelected);
  const toggleLock = useStore((s) => s.toggleLock);
  const current = useStore((s) => s.current);
  const annos = useStore((s) => s.annos);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const open = (e: Event) => {
      const { index, x, y, locked } = (
        e as CustomEvent<{ index: number; x: number; y: number; locked: boolean }>
      ).detail;
      setMenu({ index, x, y, locked });
    };
    const close = () => setMenu(null);
    window.addEventListener('uitag-relabel', open);
    window.addEventListener('pointerdown', close);
    window.addEventListener('blur', close);
    window.addEventListener('keydown', (e) => e.key === 'Escape' && close());
    return () => {
      window.removeEventListener('uitag-relabel', open);
      window.removeEventListener('pointerdown', close);
      window.removeEventListener('blur', close);
    };
  }, []);

  // 贴边收敛：菜单不出视口
  useLayoutEffect(() => {
    if (!menu || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    const nx = Math.min(menu.x, window.innerWidth - r.width - 8);
    const ny = Math.min(menu.y, window.innerHeight - r.height - 8);
    if (nx !== menu.x || ny !== menu.y) setMenu({ ...menu, x: nx, y: ny });
  }, [menu]);

  if (!menu) return null;
  const box = current ? (annos[current] ?? [])[menu.index] : undefined;

  return (
    <div
      ref={ref}
      className="animate-in fade-in-0 zoom-in-95 fixed z-50 min-w-44 overflow-hidden rounded-xl border border-border/60 bg-popover/75 p-1 text-popover-foreground shadow-2xl shadow-black/60 backdrop-blur-2xl"
      style={{ left: menu.x, top: menu.y }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {menu.locked ? (
        <button
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-accent"
          onClick={() => {
            toggleLock(menu.index);
            setMenu(null);
          }}
        >
          <IconLockOpen className="size-3.5" />
          解锁
        </button>
      ) : (
        <>
          <button
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-accent"
            onClick={() => {
              toggleLock(menu.index);
              setMenu(null);
            }}
          >
            <IconLock className="size-3.5" />
            锁定
          </button>
          <div className="my-1 h-px bg-border" />
          <p className="px-2 py-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
            改为标签
          </p>
          {tags.map((t) => (
            <button
              key={t.name}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-accent"
              onClick={() => {
                relabelBox(menu.index, t.name);
                setMenu(null);
              }}
            >
              <span className="size-2.5 rounded-full" style={{ backgroundColor: t.color }} />
              <span className="flex-1">{t.label}</span>
              {box?.tag === t.name && (
                <span
                  className="size-2 rounded-full"
                  style={{ backgroundColor: t.color, boxShadow: `0 0 0 2px ${t.color}40` }}
                />
              )}
            </button>
          ))}
          <div className="my-1 h-px bg-border" />
          <button
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-destructive transition-colors hover:bg-destructive/10"
            onClick={() => {
              deleteSelected();
              setMenu(null);
            }}
          >
            <IconTrash className="size-3.5" />
            删除此框
          </button>
        </>
      )}
    </div>
  );
}
