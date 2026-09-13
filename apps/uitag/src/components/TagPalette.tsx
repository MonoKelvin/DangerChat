import { useStore } from '../store';
import { cn } from '@/lib/utils';

/** 标签选择器：色点 + 名称 + 数字快捷键。 */
export function TagPalette() {
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const setActiveTag = useStore((s) => s.setActiveTag);
  const current = useStore((s) => s.current);
  const annos = useStore((s) => s.annos);

  const counts = current ? (annos[current] ?? []) : [];
  const countOf = (name: string) => counts.filter((b) => b.tag === name).length;

  return (
    <div className="flex items-center gap-1 border-b bg-card px-3 py-2">
      {tags.map((t, i) => {
        const active = activeTag === t.name;
        const n = countOf(t.name);
        return (
          <button
            key={t.name}
            onClick={() => setActiveTag(t.name)}
            title={`${t.label}　快捷键 ${i + 1}`}
            className={cn(
              'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] transition-all duration-150',
              active
                ? 'bg-accent font-medium text-accent-foreground shadow-[inset_0_0_0_1px_var(--border)]'
                : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
            )}
          >
            <span
              className={cn(
                'size-2.5 rounded-full transition-transform duration-150',
                active && 'scale-110 ring-2 ring-offset-1 ring-offset-card',
              )}
              style={{ backgroundColor: t.color, ...(active ? { boxShadow: `0 0 0 2px ${t.color}40` } : {}) }}
            />
            {t.label}
            {n > 0 && (
              <span className="rounded bg-foreground/10 px-1 text-[11px] tabular-nums">{n}</span>
            )}
            <kbd
              className={cn(
                'ml-0.5 rounded border px-1 text-[11px] leading-tight',
                active ? 'border-foreground/20 text-muted-foreground' : 'border-transparent text-muted-foreground/50',
              )}
            >
              {i + 1}
            </kbd>
          </button>
        );
      })}
    </div>
  );
}
