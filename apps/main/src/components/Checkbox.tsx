import { cn } from '../lib/utils';

interface CheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
}

/** 复选框：无边框依赖色，选中态用主色填充 + 白勾，尺寸与文本行对齐。 */
export function Checkbox({ checked, onChange, disabled, label }: CheckboxProps) {
  const box = (
    <span
      className={cn(
        'flex size-4 shrink-0 items-center justify-center rounded-[5px] transition-all duration-150',
        checked
          ? 'bg-[var(--brand)] text-[var(--brand-text)]'
          : 'bg-[var(--input-bg)] text-transparent',
      )}
    >
      <svg viewBox="0 0 16 16" className="size-3" fill="none" aria-hidden>
        <path
          d="M3.5 8.5 6.5 11.5 12.5 4.5"
          stroke="currentColor"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  );

  const buttonCls = cn(
    'flex cursor-pointer items-center gap-2 text-left disabled:pointer-events-none disabled:opacity-40',
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)] focus-visible:ring-offset-1',
    label ? 'text-sm text-[var(--text-primary)]' : 'rounded-[5px]',
  );

  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={buttonCls}
    >
      {box}
      {label}
    </button>
  );
}
