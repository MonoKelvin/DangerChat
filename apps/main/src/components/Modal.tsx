import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { cn } from '../lib/utils';

/**
 * 通用模态对话框（portal 挂 body，浮层语言与 Combobox/Tooltip 一致）。
 *
 * 行为约定：
 * - `Esc` 关闭（可经 `closeOnEsc={false}` 关闭）；点击遮罩关闭（同理由 `closeOnOverlay` 控制）。
 * - 打开时聚焦面板（`tabIndex={-1}`），关闭后焦点还原到触发元素。
 * - 不做焦点陷阱（本项目无复杂表单嵌套场景，保持轻量）。
 */
interface ModalProps {
  open: boolean;
  onClose: () => void;
  /** 面板标题（省略则只渲染内容） */
  title?: string;
  /** 底部操作区（右对齐；按钮由调用方提供） */
  footer?: ReactNode;
  /** 面板宽度档位，默认 md */
  size?: 'sm' | 'md' | 'lg';
  closeOnEsc?: boolean;
  closeOnOverlay?: boolean;
  children: ReactNode;
}

const SIZE_CLS: Record<NonNullable<ModalProps['size']>, string> = {
  sm: 'max-w-sm',
  md: 'max-w-md',
  lg: 'max-w-lg',
};

export function Modal({
  open,
  onClose,
  title,
  footer,
  size = 'md',
  closeOnEsc = true,
  closeOnOverlay = true,
  children,
}: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  // 打开前的焦点元素：关闭后还原，避免焦点丢失到 body
  const restoreRef = useRef<HTMLElement | null>(null);
  // onClose 经 ref 传递：调用方常传内联箭头函数，若入依赖数组会让 effect
  // 每次渲染重跑——cleanup 里的焦点还原会把焦点反复弹回触发元素（点击闪烁）。
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    restoreRef.current = document.activeElement as HTMLElement | null;
    panelRef.current?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (closeOnEsc && e.key === 'Escape') {
        e.stopPropagation();
        onCloseRef.current();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', onKey);
      restoreRef.current?.focus?.();
    };
  }, [open, closeOnEsc]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-[100] flex items-center justify-center p-4">
      {/* 遮罩：高斯模糊背景 + 点击关闭 */}
      <div
        className="absolute inset-0 bg-[var(--modal-scrim)] backdrop-blur-md"
        onPointerDown={closeOnOverlay ? onClose : undefined}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        className={cn(
          'animate-in fade-in-0 zoom-in-95 relative w-full rounded-2xl border border-[var(--glass-border)]',
          'bg-[var(--panel-bg)] shadow-[var(--shadow-lg)] outline-none',
          SIZE_CLS[size],
        )}
      >
        {title && (
          <div className="flex items-start justify-between gap-4 px-5 pt-4 pb-1">
            <h2 className="text-base font-semibold text-[var(--text-primary)]">{title}</h2>
            <button
              type="button"
              aria-label="关闭"
              onClick={onClose}
              className="-mr-1 -mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-tertiary)] transition-colors hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]"
            >
              <X className="size-4" strokeWidth={2} />
            </button>
          </div>
        )}
        <div className="px-5 py-3 text-sm leading-relaxed text-[var(--text-secondary)]">
          {children}
        </div>
        {footer && (
          <div className="flex items-center justify-end gap-2 px-5 pt-1 pb-4">{footer}</div>
        )}
      </div>
    </div>,
    document.body,
  );
}

/** 对话框按钮：primary = 主操作（实心主色），其余为次级（浅底 + 悬停提亮）。 */
export function ModalButton({
  variant = 'default',
  className,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'default' }) {
  return (
    <button
      type="button"
      className={cn(
        'h-9 rounded-lg px-3.5 text-sm font-medium transition-colors',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
        'disabled:pointer-events-none disabled:opacity-40',
        variant === 'primary'
          ? 'bg-[var(--brand)] text-[var(--brand-text)] shadow-[var(--shadow-sm)] hover:bg-[var(--brand-hover)]'
          : 'bg-[var(--card-bg)] text-[var(--text-secondary)] shadow-[var(--shadow-sm)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
        className,
      )}
      {...props}
    />
  );
}
