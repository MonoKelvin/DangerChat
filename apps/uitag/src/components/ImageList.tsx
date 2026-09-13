import { useMemo } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useStore } from '../store';

/** 左侧图片列表：缩略图 + 文件名 + 已标注标签色点（FR-TAG-03）。 */
export function ImageList() {
  const images = useStore((s) => s.images);
  const current = useStore((s) => s.current);
  const setCurrent = useStore((s) => s.setCurrent);
  const annos = useStore((s) => s.annos);
  const tags = useStore((s) => s.tags.tags);

  const colorOf = useMemo(() => {
    const m = new Map(tags.map((t) => [t.name, t.color]));
    return (tag: string) => m.get(tag) ?? '#888888';
  }, [tags]);

  return (
    <div className="flex h-full w-60 shrink-0 flex-col border-r border-border">
      <div className="flex items-center justify-between px-3 py-2 text-xs text-muted-foreground">
        <span>图片列表</span>
        <span>
          {images.length} 张 ·{' '}
          {images.filter((i) => (annos[i.path]?.length ?? 0) > 0).length} 已标注
        </span>
      </div>
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {images.map((img) => {
          const boxes = annos[img.path] ?? [];
          const used = [...new Set(boxes.map((b) => b.tag))];
          return (
            <button
              key={img.path}
              onClick={() => setCurrent(img.path)}
              className={`mb-1 flex w-full items-center gap-2 rounded-md p-1.5 text-left text-sm transition-colors ${
                current === img.path ? 'bg-accent' : 'hover:bg-muted/60'
              }`}
            >
              <img
                src={convertFileSrc(img.path)}
                alt=""
                className="h-10 w-14 shrink-0 rounded object-cover"
                loading="lazy"
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate" title={img.file_name}>
                  {img.file_name}
                </span>
                <span className="mt-0.5 flex gap-1">
                  {used.map((t) => (
                    <span
                      key={t}
                      className="size-2 rounded-full"
                      style={{ backgroundColor: colorOf(t) }}
                    />
                  ))}
                </span>
              </span>
            </button>
          );
        })}
        {images.length === 0 && (
          <p className="px-2 py-8 text-center text-sm text-muted-foreground">
            尚未导入图片
          </p>
        )}
      </div>
    </div>
  );
}
