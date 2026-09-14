import { convertFileSrc } from '@tauri-apps/api/core';
import { useStore } from '../store';

/** 多选 grid 视图：所选图片平铺展示，标题行显示已选/已标注数量，
 *  每项下方展示文件名、尺寸、标注框数（尺寸懒加载实测）。 */
export function ImageGrid() {
  const images = useStore((s) => s.images);
  const annos = useStore((s) => s.annos);
  const dims = useStore((s) => s.dims);
  const selection = useStore((s) => s.selection);
  const current = useStore((s) => s.current);
  const setDim = useStore((s) => s.setDim);

  const sel = selection.length > 0 ? selection : current ? [current] : [];
  const items = sel
    .map((p) => images.find((i) => i.path === p))
    .filter((i): i is NonNullable<typeof i> => i != null);
  const annotated = items.filter((i) => (annos[i.path]?.length ?? 0) > 0).length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border/60 bg-card px-4 py-2.5 text-[13px]">
        <span className="font-medium">已选 {items.length} 张</span>
        <span className="text-muted-foreground/40">·</span>
        <span className="text-muted-foreground">已标注 {annotated} 张</span>
        <span className="ml-auto text-xs text-muted-foreground/60">Esc 退出多选</span>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3">
          {items.map((img) => {
            const d = dims[img.path];
            const n = annos[img.path]?.length ?? 0;
            return (
              <figure
                key={img.path}
                className="overflow-hidden rounded-lg border border-border/60 bg-card transition-shadow hover:shadow-md"
              >
                <div className="flex h-40 items-center justify-center bg-canvas p-2">
                  <img
                    src={convertFileSrc(img.path)}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    className="max-h-full max-w-full object-contain"
                    onLoad={(e) => {
                      if (!dims[img.path]) {
                        setDim(img.path, e.currentTarget.naturalWidth, e.currentTarget.naturalHeight);
                      }
                    }}
                  />
                </div>
                <figcaption className="space-y-0.5 px-2.5 py-2">
                  <p className="truncate text-xs font-medium" title={img.file_name}>
                    {img.file_name}
                  </p>
                  <p className="text-[11px] tabular-nums text-muted-foreground">
                    {d ? `${d.width}×${d.height}` : '…'} · {n} 框
                  </p>
                </figcaption>
              </figure>
            );
          })}
        </div>
      </div>
    </div>
  );
}
