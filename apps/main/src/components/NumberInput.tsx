import { useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { cn } from '../lib/utils';

interface NumberInputProps {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  /** 步长（浮点输入的小数位数由步长推导） */
  step?: number;
  disabled?: boolean;
  className?: string;
}

function decimalsOf(step: number): number {
  const s = Number(step.toFixed(12)).toString();
  const dot = s.indexOf('.');
  return dot === -1 ? 0 : s.length - dot - 1;
}

/** 数字输入（参考 Wanwu WwNumberInput）：右侧堆叠步进按钮 + 悬停滚轮调节 + 范围钳制。 */
export function NumberInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  disabled,
  className,
}: NumberInputProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [focused, setFocused] = useState(false);
  const [draft, setDraft] = useState<string | null>(null);

  const clamp = (v: number) => {
    let n = v;
    if (min != null) n = Math.max(min, n);
    if (max != null) n = Math.min(max, n);
    const d = decimalsOf(step);
    return d > 0 ? Number(n.toFixed(d)) : Math.round(n);
  };

  const adjust = (dir: 1 | -1) => onChange(clamp(value + dir * step));

  // 悬停/聚焦时滚轮调节
  useEffect(() => {
    const root = rootRef.current;
    if (!root || disabled) return;
    const onWheel = (e: WheelEvent) => {
      const inside = root.matches(':hover') || root.contains(document.activeElement);
      if (!inside || e.deltaY === 0) return;
      e.preventDefault();
      const dir = e.deltaY < 0 ? 1 : -1;
      const next = clamp(value + dir * step);
      if (next !== value) onChange(next);
    };
    root.addEventListener('wheel', onWheel, { passive: false });
    return () => root.removeEventListener('wheel', onWheel);
  });

  const display = draft ?? String(value);

  return (
    <div
      ref={rootRef}
      className={cn(
        'flex h-9 items-center rounded-lg bg-[var(--input-bg)] transition-all duration-150',
        'focus-within:bg-[var(--panel-bg)] focus-within:shadow-[inset_0_0_0_1.5px_var(--brand)]',
        'hover:bg-[var(--active-overlay)] focus-within:hover:bg-[var(--panel-bg)]',
        disabled && 'pointer-events-none opacity-40',
        className ?? 'w-36',
      )}
    >
      <input
        type="text"
        inputMode="decimal"
        value={display}
        disabled={disabled}
        onFocus={() => {
          setFocused(true);
          setDraft(String(value));
        }}
        onChange={(e) => setDraft(e.target.value.replace(/[^\d.-]/g, ''))}
        onBlur={() => {
          setFocused(false);
          const n = parseFloat(draft ?? '');
          setDraft(null);
          if (Number.isFinite(n) && clamp(n) !== value) onChange(clamp(n));
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
          if (e.key === 'ArrowUp') {
            e.preventDefault();
            onChange(clamp((focused ? parseFloat(draft ?? '0') || 0 : value) + step));
          }
          if (e.key === 'ArrowDown') {
            e.preventDefault();
            onChange(clamp((focused ? parseFloat(draft ?? '0') || 0 : value) - step));
          }
        }}
        className="h-full min-w-0 flex-1 bg-transparent px-3 text-right text-sm tabular-nums text-[var(--text-primary)] outline-none"
      />
      {/* 堆叠步进按钮：lucide chevron 字形只占图标视口中间 1/4，两侧留白大，
          单纯缩小按钮压不下箭头间隔——需负边距让两个按钮的盒体交叠 */}
      <div className="flex h-full flex-col justify-center">
        <button
          type="button"
          tabIndex={-1}
          onClick={() => adjust(1)}
          className="flex h-4 items-center justify-center pl-0.5 pr-1.5 text-[var(--text-tertiary)] transition-colors hover:text-[var(--text-primary)]"
        >
          <ChevronUp className="size-4" strokeWidth={2.5} />
        </button>
        <button
          type="button"
          tabIndex={-1}
          onClick={() => adjust(-1)}
          className="-mt-[5px] flex h-4 items-center justify-center pl-0.5 pr-1.5 text-[var(--text-tertiary)] transition-colors hover:text-[var(--text-primary)]"
        >
          <ChevronDown className="size-4" strokeWidth={2.5} />
        </button>
      </div>
    </div>
  );
}
