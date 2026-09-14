import { useRef, useState } from 'react';
import { IconCircleCheck, IconSparkles } from '@tabler/icons-react';
import { useStore } from '../store';
import { Button } from './ui/button';
import { Checkbox } from './ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { cn } from '@/lib/utils';

/** 自动预标注对话框：确认 → 进度 → 结果；可选是否覆盖已有标注。 */
export function PropagateDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
}) {
  const current = useStore((s) => s.current);
  const images = useStore((s) => s.images);
  const annos = useStore((s) => s.annos);
  const propagating = useStore((s) => s.propagating);
  const propagateToAll = useStore((s) => s.propagateToAll);

  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ applied: number; total: number } | null>(null);

  // 重新打开时在渲染期同步复位（关闭时保持结果页直到退场动画播完，
  // 避免 useEffect 立即重置导致关闭动画期间闪现确认页）
  const wasOpen = useRef(false);
  if (open && !wasOpen.current) {
    wasOpen.current = true;
    setBusy(false);
    setError(null);
    setResult(null);
    setOverwrite(false);
  }
  if (!open) wasOpen.current = false;

  const boxes = current ? (annos[current] ?? []) : [];
  const others = images.filter((i) => i.path !== current);
  const unlabeled = others.filter((i) => (annos[i.path]?.length ?? 0) === 0).length;
  const targetCount = overwrite ? others.length : unlabeled;

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await propagateToAll(overwrite));
    } catch (e) {
      setError(e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = (v: boolean) => {
    if (busy && !v) return; // 进行中不允许关闭
    onOpenChange(v);
  };

  const pct = propagating ? Math.round((propagating.done / propagating.total) * 100) : 0;

  return (
    <Dialog open={open} onOpenChange={reset}>
      <DialogContent className="sm:max-w-sm" showCloseButton={!busy}>
        {result ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <IconCircleCheck className="size-4 text-emerald-500" />
                预标注完成
              </DialogTitle>
              <DialogDescription>
                {result.total > 0
                  ? `${result.applied}/${result.total} 张图片获得标注，请逐张检查修正`
                  : '没有需要处理的图片'}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button onClick={() => reset(false)}>完成</Button>
            </DialogFooter>
          </>
        ) : busy ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <span className="size-4 animate-spin rounded-full border-2 border-current border-t-transparent text-primary" />
                匹配中
              </DialogTitle>
              <DialogDescription>
                {propagating ? `正在匹配 ${propagating.done}/${propagating.total} 张…` : '准备中…'}
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-2">
              <div className="h-1.5 overflow-hidden rounded-full bg-foreground/10">
                <div
                  className="h-full rounded-full bg-primary transition-all duration-200"
                  style={{ width: `${pct}%` }}
                />
              </div>
              <p className="text-right text-xs tabular-nums text-muted-foreground">{pct}%</p>
            </div>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <IconSparkles className="size-4 text-primary" />
                自动预标注
              </DialogTitle>
              <DialogDescription>
                以当前图的 {boxes.length} 个标注为模板，匹配推算其余图片，结果需人工复核。
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-3">
              <div className="flex items-center justify-between rounded-lg border bg-muted/30 px-3.5 py-3">
                <span className="text-xs text-muted-foreground">待处理</span>
                <span className="text-sm font-semibold tabular-nums">
                  {targetCount}
                  <span className="ml-0.5 text-xs font-normal text-muted-foreground">张</span>
                </span>
              </div>
              {/* 不用 <label>：label 会把点击转发给内部 button（Radix Checkbox），
                  与外层 onClick 叠加导致勾选状态来回抵消 */}
              <div
                className={cn(
                  'flex cursor-pointer items-center gap-2.5 rounded-lg border px-3.5 py-3 transition-colors',
                  overwrite ? 'border-primary/60 bg-primary/5' : 'bg-muted/30 hover:bg-muted/50',
                )}
                onClick={() => setOverwrite((v) => !v)}
              >
                <Checkbox checked={overwrite} className="pointer-events-none" tabIndex={-1} />
                <span className="flex-1">
                  <span className="block text-[13px]">覆盖已有标注</span>
                  <span className="block text-xs text-muted-foreground">
                    不勾选时仅处理未标注的图片
                  </span>
                </span>
              </div>
            </div>

            {error && (
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">{error}</p>
            )}

            <DialogFooter>
              <Button variant="ghost" onClick={() => reset(false)}>
                取消
              </Button>
              <Button disabled={targetCount === 0 || boxes.length === 0} onClick={run}>
                开始
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
