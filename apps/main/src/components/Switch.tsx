import { cn } from '../lib/utils';

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
}

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
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)] focus-visible:ring-offset-1',
        'disabled:pointer-events-none disabled:opacity-40',
        checked
          ? 'bg-[var(--primary)] shadow-[var(--shadow-sm)]'
          : 'bg-[var(--text-tertiary)]',
      )}
    >
      <span
        className={cn(
          'absolute top-0.5 size-5 rounded-full bg-white shadow-md transition-all duration-200',
          'group-hover:shadow-lg',
          checked ? 'left-[22px]' : 'left-0.5',
        )}
      />
    </button>
  );
}
