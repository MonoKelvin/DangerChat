import { useState } from 'react';
import { IconSparkles, IconTrash } from '@tabler/icons-react';
import { useStore } from '../store';
import { cn } from '@/lib/utils';
import { Button } from './ui/button';
import { PropagateDialog } from './PropagateDialog';

/** 标签选择器：色点 + 名称 + 数字快捷键；右端为「自动预标注」图标按钮。 */
export function TagPalette() {
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const setActiveTag = useStore((s) => s.setActiveTag);
  const current = useStore((s) => s.current);
  const annos = useStore((s) => s.annos);
  const images = useStore((s) => s.images);
  const propagating = useStore((s) => s.propagating);
  const clearCurrent = useStore((s) => s.clearCurrent);

  const counts = current ? (annos[current] ?? []) : [];
  const countOf = (name: string) => counts.filter((b) => b.tag === name).length;
  const canPropagate = current && counts.length > 0 && propagating == null && images.length >= 2;
  const [showPropagate, setShowPropagate] = useState(false);

  return (
    <div className="flex items-center gap-1 border-b bg-card px-3 py-2">
      <Button
        variant="ghost"
        size="icon"
        className="size-8 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
        disabled={counts.length === 0}
        onClick={clearCurrent}
        data-tip="清空当前图片的标注"
      >
        <IconTrash className="size-4" />
      </Button>
      <div className="mx-1 h-5 w-px shrink-0 bg-border" />
      {tags.map((t, i) => {
        const active = activeTag === t.name;
        const n = countOf(t.name);
        return (
          <button
            key={t.name}
            onClick={() => setActiveTag(t.name)}
            data-tip={`${t.label}（快捷键 ${i + 1}）`}
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
          </button>
        );
      })}
      <div className="flex-1" />
      <Button
        variant="ghost"
        size="icon"
        className="size-8 text-primary"
        disabled={!canPropagate}
        onClick={() => setShowPropagate(true)}
        data-tip={
          propagating != null
            ? `匹配中 ${propagating.done}/${propagating.total} 张…`
            : '自动预标注：以当前图标注为模板匹配其余图片，结果需人工复核'
        }
      >
        {propagating != null ? (
          <span className="size-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
        ) : (
          <IconSparkles className="size-4" />
        )}
      </Button>
      <PropagateDialog open={showPropagate} onOpenChange={setShowPropagate} />
    </div>
  );
}
