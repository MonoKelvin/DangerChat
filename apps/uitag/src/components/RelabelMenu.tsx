import { useEffect, useRef, useState } from 'react';
import { useStore } from '../store';

declare global {
  interface Window {
    lastMouseX?: number;
    lastMouseY?: number;
  }
}

/** 右键改标签菜单：AnnoCanvas 派发 uitag-relabel 事件，此处渲染浮层。 */
export function RelabelMenu() {
  const tags = useStore((s) => s.tags.tags);
  const relabelBox = useStore((s) => s.relabelBox);
  const [pos, setPos] = useState<{ x: number; y: number; index: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const open = (e: Event) => {
      const me = e as CustomEvent<{ index: number }>;
      setPos({
        x: (window.lastMouseX ?? 100) - 8,
        y: (window.lastMouseY ?? 100) - 8,
        index: me.detail.index,
      });
    };
    const track = (e: MouseEvent) => {
      window.lastMouseX = e.clientX;
      window.lastMouseY = e.clientY;
    };
    const close = () => setPos(null);
    window.addEventListener('uitag-relabel', open);
    window.addEventListener('mousemove', track);
    window.addEventListener('click', close);
    return () => {
      window.removeEventListener('uitag-relabel', open);
      window.removeEventListener('mousemove', track);
      window.removeEventListener('click', close);
    };
  }, []);

  if (!pos) return null;
  return (
    <div
      ref={ref}
      className="fixed z-50 min-w-36 overflow-hidden rounded-md border border-border bg-popover py-1 shadow-lg"
      style={{ left: pos.x, top: pos.y }}
      onClick={(e) => e.stopPropagation()}
    >
      <p className="px-3 pb-1 pt-0.5 text-xs text-muted-foreground">改为标签</p>
      {tags.map((t) => (
        <button
          key={t.name}
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-accent"
          onClick={() => {
            relabelBox(pos.index, t.name);
            setPos(null);
          }}
        >
          <span className="size-3 rounded-sm" style={{ backgroundColor: t.color }} />
          {t.label}
        </button>
      ))}
    </div>
  );
}
