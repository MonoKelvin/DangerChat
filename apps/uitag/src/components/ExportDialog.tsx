import { useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { useStore } from '../store';

/** 导出对话框：选目标路径 → export_zip → 展示结果。 */
export function ExportDialog({ onClose }: { onClose: () => void }) {
  const exportAll = useStore((s) => s.exportAll);
  const images = useStore((s) => s.images);
  const annos = useStore((s) => s.annos);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const labeled = images.filter((i) => (annos[i.path]?.length ?? 0) > 0).length;

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const dest = await save({
        title: '导出 YOLO 训练集',
        defaultPath: 'uitag-dataset.zip',
        filters: [{ name: 'zip', extensions: ['zip'] }],
      });
      if (!dest) {
        setBusy(false);
        return;
      }
      const path = await exportAll(dest);
      setResult(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/40">
      <div className="w-96 rounded-lg bg-background p-5 shadow-xl">
        <h2 className="mb-3 text-base font-semibold">导出训练集</h2>
        {result ? (
          <div className="space-y-3">
            <p className="text-sm text-green-600">导出完成</p>
            <p className="break-all rounded bg-muted p-2 text-xs text-muted-foreground">
              {result}
            </p>
            <button
              className="w-full rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground"
              onClick={onClose}
            >
              关闭
            </button>
          </div>
        ) : (
          <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
              共 {images.length} 张图片，{labeled} 张已标注（未标注的作为负样本导出）。
            </p>
            <p className="text-xs text-muted-foreground">
              产出：images/ + labels/（YOLO 归一化）+ classes.txt + tags.json
            </p>
            {error && <p className="text-sm text-red-500">{error}</p>}
            <div className="flex gap-2">
              <button
                disabled={busy || images.length === 0}
                className="flex-1 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
                onClick={run}
              >
                {busy ? '导出中…' : '选择位置并导出'}
              </button>
              <button
                className="rounded-md border border-border px-3 py-2 text-sm"
                onClick={onClose}
              >
                取消
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
