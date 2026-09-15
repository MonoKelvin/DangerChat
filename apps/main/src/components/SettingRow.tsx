import { type ReactNode } from 'react';
import { cn } from '../lib/utils';

interface SettingRowProps {
  label: string;
  description?: string;
  children: ReactNode;
  error?: string;
  className?: string;
}

export function SettingRow({ label, description, children, error, className }: SettingRowProps) {
  return (
    <div className={cn('flex items-center justify-between gap-6 rounded-lg bg-[var(--card-bg)] px-4 py-3 shadow-[var(--shadow-sm)] transition-shadow hover:shadow-[var(--shadow-md)]', className)}>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-[var(--text-primary)]">{label}</p>
        {description && (
          <p className="mt-0.5 text-xs leading-relaxed text-[var(--text-secondary)]">{description}</p>
        )}
        {error && (
          <p className="mt-1 text-xs text-[var(--danger)]">{error}</p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
