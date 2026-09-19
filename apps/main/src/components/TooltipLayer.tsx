import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../lib/utils';

/**
 * 全局 tooltip 层：事件委托实现，main.tsx 挂载一次即可。
 * 任意元素加属性即生效（替代原生 title，统一样式）：
 *   data-tip="文本"            —— 必需
 *   data-tip-hint="副行文本"    —— 可选，第二行弱化说明
 *   data-tip-side="top|bottom|left|right" —— 可选，默认 bottom
 *   data-tip-class="附加类"     —— 可选
 *   data-tip-delay="毫秒"       —— 可选，覆盖默认 400ms 悬停延迟（如弹窗按钮设 50）
 *
 * 长文本自动换行：先按 nowrap 测自然宽度，超出可用宽度时切换 wrap 并重定位。
 */
type Side = 'top' | 'bottom' | 'left' | 'right';

type Tip = {
  text: string;
  hint?: string;
  side: Side;
  cls?: string;
  rect: DOMRect;
};

const SHOW_DELAY_MS = 400;
/** 与窗口边缘的最小间距 */
const EDGE = 12;

export function TooltipLayer() {
  const [tip, setTip] = useState<Tip | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [wrap, setWrap] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    const hide = () => {
      window.clearTimeout(timer.current);
      setTip(null);
    };

    const onOver = (e: MouseEvent) => {
      const el = (e.target as Element | null)?.closest?.('[data-tip]') as HTMLElement | null;
      const text = el?.getAttribute('data-tip');
      if (el && text) {
        window.clearTimeout(timer.current);
        const delayAttr = el.getAttribute('data-tip-delay');
        const delay = delayAttr != null ? Math.max(0, Number(delayAttr)) : SHOW_DELAY_MS;
        timer.current = window.setTimeout(() => {
          setWrap(false);
          setTip({
            text,
            hint: el.getAttribute('data-tip-hint') ?? undefined,
            cls: el.getAttribute('data-tip-class') ?? undefined,
            side: (el.getAttribute('data-tip-side') as Side) || 'bottom',
            rect: el.getBoundingClientRect(),
          });
        }, delay);
      } else {
        hide();
      }
    };

    document.addEventListener('mouseover', onOver);
    document.addEventListener('mousedown', hide);
    window.addEventListener('scroll', hide, true);
    window.addEventListener('resize', hide);
    window.addEventListener('blur', hide);
    return () => {
      window.clearTimeout(timer.current);
      document.removeEventListener('mouseover', onOver);
      document.removeEventListener('mousedown', hide);
      window.removeEventListener('scroll', hide, true);
      window.removeEventListener('resize', hide);
      window.removeEventListener('blur', hide);
    };
  }, []);

  // 渲染后按实测尺寸定位（offsetWidth 不受入场 scale 变换影响）
  useLayoutEffect(() => {
    if (!tip || !boxRef.current) return;
    const box = boxRef.current;
    const VW = window.innerWidth;
    const VH = window.innerHeight;
    const maxW = VW - EDGE * 2;

    if (!wrap && box.scrollWidth > maxW) {
      setWrap(true);
      return;
    }

    const width = box.offsetWidth;
    const height = box.offsetHeight;
    const r = tip.rect;
    const gap = 8;
    const clampH = (left: number) => Math.min(Math.max(left, EDGE), VW - width - EDGE);
    const clampV = (top: number) => Math.min(Math.max(top, EDGE), VH - height - EDGE);

    let left: number;
    let top: number;
    switch (tip.side) {
      case 'left':
        left = r.left - width - gap;
        if (left < EDGE) left = r.right + gap;
        top = r.top + r.height / 2 - height / 2;
        break;
      case 'right':
        left = r.right + gap;
        if (left + width > VW - EDGE) left = r.left - width - gap;
        top = r.top + r.height / 2 - height / 2;
        break;
      case 'top':
        top = r.top - height - gap;
        if (top < EDGE) top = r.bottom + gap;
        left = r.left + r.width / 2 - width / 2;
        break;
      default:
        top = r.bottom + gap;
        if (top + height > VH - EDGE) top = r.top - height - gap;
        left = r.left + r.width / 2 - width / 2;
    }
    left = clampH(left);
    top = clampV(top);
    setPos({ left, top });
  }, [tip, wrap]);

  if (!tip) return null;

  return createPortal(
    <div
      ref={boxRef}
      className={cn(
        'animate-in fade-in-0 zoom-in-95 pointer-events-none fixed z-[100] rounded-lg bg-[var(--tooltip-bg)] px-2.5 py-1.5 text-xs text-[var(--tooltip-text)] shadow-[var(--shadow-lg)]',
        wrap ? 'max-w-[calc(100vw-24px)] whitespace-normal' : 'w-max whitespace-nowrap',
        tip.cls,
      )}
      style={{ left: pos?.left ?? -9999, top: pos?.top ?? -9999 }}
    >
      {tip.text}
      {tip.hint && <p className="mt-1 text-caption opacity-60">{tip.hint}</p>}
    </div>,
    document.body,
  );
}
