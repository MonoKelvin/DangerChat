import { useStore } from '../store';

/** 标签选择面板：色块 + 名称，1-4 快捷键对应。 */
export function TagPalette() {
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const setActiveTag = useStore((s) => s.setActiveTag);

  return (
    <div className="flex flex-wrap gap-1.5 p-2">
      {tags.map((t, i) => (
        <button
          key={t.name}
          onClick={() => setActiveTag(t.name)}
          className={`flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-sm transition-colors ${
            activeTag === t.name
              ? 'border-foreground/40 bg-accent font-medium'
              : 'border-transparent bg-muted/50 hover:bg-muted'
          }`}
          title={`快捷键 ${i + 1}`}
        >
          <span className="size-3 rounded-sm" style={{ backgroundColor: t.color }} />
          {t.label}
          <kbd className="rounded bg-background/70 px-1 text-xs text-muted-foreground">
            {i + 1}
          </kbd>
        </button>
      ))}
    </div>
  );
}
