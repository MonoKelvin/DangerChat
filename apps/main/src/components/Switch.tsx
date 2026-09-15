import { cn } from '../lib/utils';

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
}

/** 开关：选中态品牌色填充，未选中 zinc 灰，无边框依赖。 */
export function Switch({ checked, onChange, disabled, label }: SwitchProps) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'group relative h-6 w-11 shrink-0 rounded-full transition-all duration-200',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--panel-bg)]',
        'disabled:pointer-events-none disabled:opacity-40',
        checked ? 'bg-[var(--brand)]' : 'bg-[var(--input-bg)]',
      )}
    >
      <span
        className={cn(
          'absolute top-0.5 size-5 rounded-full shadow-sm transition-all duration-200',
          checked ? 'left-[22px] bg-[var(--brand-text)]' : 'left-0.5 bg-[var(--text-tertiary)]',
        )}
      />
    </button>
  );
}
