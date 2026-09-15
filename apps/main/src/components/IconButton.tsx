import { type ButtonHTMLAttributes, forwardRef } from 'react';
import { cn } from '../lib/utils';

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'default' | 'ghost' | 'danger' | 'primary';
  size?: 'sm' | 'md';
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  ({ className, variant = 'default', size = 'md', ...props }, ref) => {
    return (
      <button
        ref={ref}
        className={cn(
          'inline-flex items-center justify-center rounded-lg transition-all duration-150',
          'disabled:pointer-events-none disabled:opacity-40',
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
          size === 'sm' && 'size-8 text-sm',
          size === 'md' && 'size-10 text-base',
          variant === 'default' && 'hover:bg-[var(--hover-overlay)]',
          variant === 'ghost' && 'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
          variant === 'primary' && 'bg-[var(--primary)] text-[var(--primary-text)] hover:bg-[var(--primary-hover)] shadow-[var(--shadow-sm)]',
          variant === 'danger' && 'hover:bg-[var(--danger)] hover:text-white',
          className,
        )}
        {...props}
      />
    );
  },
);

IconButton.displayName = 'IconButton';
