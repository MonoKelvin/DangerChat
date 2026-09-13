import { useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { CheckCircle2, FileArchive, Loader2 } from 'lucide-react';
import { useStore } from '../store';
import { Button } from './ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';

/** 导出对话框：选目标路径 → export_zip → 展示结果。 */
export function ExportDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
}) {
  const exportAll = useStore((s) => s.exportAll);
  const images = useStore((s) => s.images);
  const annos = useStore((s) => s.annos);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const labeled = images.filter((i) => (annos[i.path]?.length ?? 0) > 0).length;
  const boxCount = Object.values(annos).reduce((n, b) => n + b.length, 0);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const dest = await save({
        title: '导出 YOLO 训练集',
        defaultPath: 'uitag-dataset.zip',
        filters: [{ name: 'ZIP 压缩包', extensions: ['zip'] }],
      });
      if (!dest) return;
      setResult(await exportAll(dest));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = (v: boolean) => {
    if (!v) {
      setResult(null);
      setError(null);
    }
    onOpenChange(v);
  };

  return (
    <Dialog open={open} onOpenChange={reset}>
      <DialogContent className="sm:max-w-md">
        {result ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <CheckCircle2 className="size-4 text-emerald-500" />
                导出完成
              </DialogTitle>
              <DialogDescription>数据集已写入下列位置。</DialogDescription>
            </DialogHeader>
            <p className="rounded-md bg-muted px-3 py-2 font-mono text-xs break-all text-muted-foreground">
              {result}
            </p>
            <DialogFooter>
              <Button onClick={() => reset(false)}>关闭</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <FileArchive className="size-4" />
                导出训练集
              </DialogTitle>
              <DialogDescription>
                产出 YOLO 格式目录结构，可直接用于 ultralytics 训练。
              </DialogDescription>
            </DialogHeader>

            <dl className="grid grid-cols-3 gap-3 text-center">
              {[
                ['图片', images.length],
                ['已标注', labeled],
                ['标注框', boxCount],
              ].map(([k, v]) => (
                <div key={k as string} className="rounded-lg border bg-muted/30 py-2.5">
                  <dd className="text-lg font-semibold tabular-nums">{v}</dd>
                  <dt className="text-[11px] text-muted-foreground">{k}</dt>
                </div>
              ))}
            </dl>

            <p className="text-xs leading-relaxed text-muted-foreground">
              <code className="text-foreground">images/</code> 原图 ·{' '}
              <code className="text-foreground">labels/</code> 归一化标注 ·{' '}
              <code className="text-foreground">classes.txt</code> ·{' '}
              <code className="text-foreground">tags.json</code>
              <br />
              未标注的图片作为负样本导出（空标注文件）。
            </p>

            {error && (
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
                {error}
              </p>
            )}

            <DialogFooter>
              <Button variant="ghost" onClick={() => reset(false)}>
                取消
              </Button>
              <Button disabled={busy || images.length === 0} onClick={run}>
                {busy && <Loader2 className="size-3.5 animate-spin" />}
                {busy ? '导出中…' : '选择位置并导出'}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
