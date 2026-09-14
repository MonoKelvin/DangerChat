import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '@/lib/utils';

/**
 * 全局 tooltip 层：事件委托实现，App 挂载一次即可。
 * 任意元素加属性即生效：
 *   data-tip="文本"            —— 必需，tooltip 内容
 *   data-tip-hint="副行文本"    —— 可选，第二行弱化说明
 *   data-tip-side="top|bottom|left|right" —— 可选，默认 bottom（空间不足自动翻转）
 *   data-tip-class="附加类"     —— 可选，如 "max-w-md whitespace-normal"
 *
 * 长文本自动换行：先按 nowrap 测自然宽度，超出窗口可用宽度时切换为 wrap
 * 并按换行后的实际尺寸重新定位，保证 tooltip 始终完整落在窗口内部。
 */
type Side = 'top' | 'bottom' | 'left' | 'right';

type Tip = {
  text: string;
  hint?: string;
  side: Side;
  cls?: string;
  rect: DOMRect;
};

const SHOW_DELAY_MS = 350;
/** 与窗口边缘的最小间距（不只贴边） */
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
        timer.current = window.setTimeout(() => {
          setWrap(false);
          setTip({
            text,
            hint: el.getAttribute('data-tip-hint') ?? undefined,
            cls: el.getAttribute('data-tip-class') ?? undefined,
            side: (el.getAttribute('data-tip-side') as Side) || 'bottom',
            rect: el.getBoundingClientRect(),
          });
        }, SHOW_DELAY_MS);
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

  // 渲染后按实测尺寸定位（默认在触发元素下方居中，空间不足时自动翻转/夹回窗口内）
  useLayoutEffect(() => {
    if (!tip || !boxRef.current) return;
    const box = boxRef.current;
    const VW = window.innerWidth;
    const VH = window.innerHeight;
    const maxW = VW - EDGE * 2;

    // 第一遍：nowrap 下自然宽度超出可用宽度 → 切换换行，类变更后本 effect 重跑再定位
    if (!wrap && box.scrollWidth > maxW) {
      setWrap(true);
      return;
    }

    // 用 offsetWidth/offsetHeight 测量：入场动画的 scale 变换会让
    // getBoundingClientRect 偏小 ~5%，按它夹边动画结束后就会超出窗口
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
        if (left < EDGE) left = r.right + gap; // 左边放不下 → 翻到右侧
        top = r.top + r.height / 2 - height / 2;
        break;
      case 'right':
        left = r.right + gap;
        if (left + width > VW - EDGE) left = r.left - width - gap; // 右边放不下 → 翻到左侧
        top = r.top + r.height / 2 - height / 2;
        break;
      case 'top':
        top = r.top - height - gap;
        if (top < EDGE) top = r.bottom + gap; // 上方放不下 → 翻到下方
        left = r.left + r.width / 2 - width / 2;
        break;
      default:
        top = r.bottom + gap;
        if (top + height > VH - EDGE) top = r.top - height - gap;
        left = r.left + r.width / 2 - width / 2;
    }
    // 全方向夹回窗口内部，且与边缘保持 EDGE 间距
    left = clampH(left);
    top = clampV(top);
    setPos({ left, top });
  }, [tip, wrap]);

  if (!tip) return null;

  return createPortal(
    <div
      ref={boxRef}
      className={cn(
        'animate-in fade-in-0 zoom-in-95 pointer-events-none fixed z-[100] rounded-lg border border-border/60 bg-popover/70 px-2.5 py-1.5 text-xs text-popover-foreground shadow-lg shadow-black/40 backdrop-blur-xl',
        wrap ? 'max-w-[calc(100vw-24px)] whitespace-normal' : 'w-max whitespace-nowrap',
        tip.cls,
      )}
      style={{ left: pos?.left ?? -9999, top: pos?.top ?? -9999 }}
    >
      {tip.text}
      {tip.hint && <p className="mt-1 text-[11px] text-muted-foreground">{tip.hint}</p>}
    </div>,
    document.body,
  );
}
