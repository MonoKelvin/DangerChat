import { useEffect, useMemo, useRef } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { IconCheck, IconPhoto } from '@tabler/icons-react';
import { useStore } from '../store';
import { cn } from '@/lib/utils';

/** 左侧图片列表：缩略图 + 已标注角标 + 上下张导航（键盘 ↑↓）。 */
export function ImageList() {
  const images = useStore((s) => s.images);
  const current = useStore((s) => s.current);
  const setCurrent = useStore((s) => s.setCurrent);
  const annos = useStore((s) => s.annos);
  const tags = useStore((s) => s.tags.tags);
  const listRef = useRef<HTMLDivElement>(null);

  // 当前图片变化（含启动时恢复上次选中）时滚动到可视区
  useEffect(() => {
    if (!current || !listRef.current) return;
    const el = listRef.current.querySelector(`[data-path="${CSS.escape(current)}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [current, images]);

  const colorOf = useMemo(() => {
    const m = new Map(tags.map((t) => [t.name, t.color]));
    return (tag: string) => m.get(tag) ?? '#94a3b8';
  }, [tags]);

  const done = images.filter((i) => (annos[i.path]?.length ?? 0) > 0).length;

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-border/60 bg-sidebar">
      {/* 头部：计数 + 进度环 */}
      <div className="flex items-center justify-between px-3 py-2.5">
        <span className="text-xs font-medium tracking-widest text-muted-foreground uppercase">
          数据集
        </span>
        <span className="flex items-center gap-1.5 text-xs tabular-nums text-muted-foreground">
          {done}/{images.length}
          <div className="h-1 w-12 overflow-hidden rounded-full bg-border/60">
            <div
              className="h-full rounded-full bg-emerald-500 transition-all duration-300"
              style={{ width: images.length ? `${(done / images.length) * 100}%` : 0 }}
            />
          </div>
        </span>
      </div>

      {/* 列表：原生 overflow（Radix ScrollArea 在 flex 链中高度塌陷导致整页滚动） */}
      {images.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2.5 px-6 text-center">
          <div className="flex size-11 items-center justify-center rounded-xl bg-accent/60">
            <IconPhoto className="size-5 text-muted-foreground" />
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            尚未打开目录
            <br />
            使用上方「打开目录」选择图片目录
          </p>
        </div>
      ) : (
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2">
          <div className="space-y-0.5">
            {images.map((img) => {
              const boxes = annos[img.path] ?? [];
              const used = [...new Set(boxes.map((b) => b.tag))];
              const active = current === img.path;
              return (
                <button
                  key={img.path}
                  data-path={img.path}
                  onClick={() => setCurrent(img.path)}
                  className={cn(
                    'flex w-full items-center gap-2.5 rounded-md p-1.5 text-left transition-all',
                    active
                      ? 'relative z-10 bg-accent shadow-[0_6px_18px_rgb(0_0_0/0.6)] ring-[inset_0_0_0_1px_var(--border)]'
                      : 'relative hover:bg-accent/50',
                  )}
                >
                  <div
                    className={cn(
                      'relative size-10 shrink-0 overflow-hidden rounded-[5px] bg-muted transition-shadow',
                      active ? 'ring-1 ring-ring/40 shadow-sm' : 'ring-1 ring-transparent',
                    )}
                  >
                    <img
                      src={convertFileSrc(img.path)}
                      alt=""
                      loading="lazy"
                      decoding="async"
                      className="size-full object-cover"
                    />
                    {boxes.length > 0 && (
                      <span className="absolute right-0 bottom-0 flex size-3.5 items-center justify-center rounded-tl-[4px] bg-emerald-500 text-white">
                        <IconCheck className="size-2.5" strokeWidth={3.5} />
                      </span>
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p
                      className={cn(
                        'truncate text-xs leading-4',
                        active
                          ? 'font-medium text-sidebar-foreground'
                          : 'text-muted-foreground',
                      )}
                      data-tip={img.file_name}
                      data-tip-side="right"
                    >
                      {img.file_name}
                    </p>
                    <div className="mt-1 flex h-1.5 items-center gap-1">
                      {used.map((t) => (
                        <span
                          key={t}
                          className="size-1.5 rounded-full"
                          style={{ backgroundColor: colorOf(t) }}
                        />
                      ))}
                      {used.length === 0 && (
                        <span className="text-[11px] text-muted-foreground/40">未标注</span>
                      )}
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </aside>
  );
}
