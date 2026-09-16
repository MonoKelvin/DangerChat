import { useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown } from 'lucide-react';
import { cn } from '../lib/utils';

export interface ComboOption {
  value: string;
  label: string;
  /** 副说明（右侧灰字） */
  hint?: string;
}

interface ComboboxProps {
  value: string;
  options: ComboOption[];
  onChange: (v: string) => void;
  disabled?: boolean;
  /** 触发器宽度 class，如 w-52 */
  className?: string;
  /** 宽度自适应面板（默认与触发器同宽） */
  panelClassName?: string;
}

/** 下拉选择：不允许输入；面板高斯模糊浮层，选中项打勾。 */
export function Combobox({
  value,
  options,
  onChange,
  disabled,
  className,
  panelClassName,
}: ComboboxProps) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);

  useLayoutEffect(() => {
    if (open && btnRef.current) setRect(btnRef.current.getBoundingClientRect());
  }, [open]);

  const selected = options.find((o) => o.value === value);

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'flex h-9 items-center justify-between gap-2 rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150',
          'hover:bg-[var(--active-overlay)] focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
          'disabled:pointer-events-none disabled:opacity-40',
          open && 'bg-[var(--group-bg)] shadow-[inset_0_0_0_1.5px_var(--brand)]',
          className ?? 'w-52',
        )}
      >
        <span className="truncate">{selected?.label ?? value}</span>
        <ChevronDown
          className={cn('size-4 shrink-0 text-[var(--text-tertiary)] transition-transform duration-200', open && 'rotate-180')}
          strokeWidth={2}
        />
      </button>

      {open &&
        rect &&
        createPortal(
          <>
            {/* 点击外部关闭（z 高于 Modal 的 z-[100]，否则面板被对话框遮住） */}
            <div className="fixed inset-0 z-[120]" onPointerDown={() => setOpen(false)} />
            <div
              role="listbox"
              className={cn(
                'animate-in fade-in-0 zoom-in-95 fixed z-[125] overflow-hidden rounded-xl border border-[var(--glass-border)] bg-[var(--popover-blur)] p-1.5 shadow-[var(--shadow-lg)] backdrop-blur-2xl',
                panelClassName,
              )}
              style={{
                left: Math.min(rect.left, window.innerWidth - (rect.width || 220) - 12),
                top: Math.min(rect.bottom + 6, window.innerHeight - 12 - options.length * 40 - 12),
                width: panelClassName ? undefined : rect.width,
              }}
            >
              {options.map((o) => (
                <button
                  key={o.value}
                  role="option"
                  aria-selected={o.value === value}
                  onClick={() => {
                    onChange(o.value);
                    setOpen(false);
                  }}
                  className={cn(
                    'flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors',
                    'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
                  )}
                >
                  <Check
                    className={cn(
                      'size-4 shrink-0 text-[var(--brand)] transition-opacity',
                      o.value === value ? 'opacity-100' : 'opacity-0',
                    )}
                    strokeWidth={2.5}
                  />
                  <span className="flex-1 truncate">{o.label}</span>
                  {o.hint && (
                    <span className="shrink-0 text-xs text-[var(--text-tertiary)]">{o.hint}</span>
                  )}
                </button>
              ))}
            </div>
          </>,
          document.body,
        )}
    </>
  );
}
