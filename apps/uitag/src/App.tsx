import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  CheckCircle2,
  FileArchive,
  FileImage,
  FolderOpen,
  Maximize2,
  Redo2,
  Sparkles,
  Undo2,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { useStore } from './store';
import { ImageList } from './components/ImageList';
import { TagPalette } from './components/TagPalette';
import { AnnoCanvas } from './components/AnnoCanvas';
import { RelabelMenu } from './components/RelabelMenu';
import { ExportDialog } from './components/ExportDialog';
import { WindowControls } from './components/WindowControls';
import { Button } from './components/ui/button';
import { Separator } from './components/ui/separator';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './components/ui/tooltip';

function ToolButton({
  icon,
  label,
  onClick,
  disabled,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" className="size-8 text-muted-foreground" onClick={onClick} disabled={disabled}>
          {icon}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

/** 传播结果 toast（自动消失）。 */
function PropagateToast() {
  const last = useStore((s) => s.lastPropagate);
  const [shown, setShown] = useState<typeof last>(null);

  useEffect(() => {
    if (last) {
      setShown(last);
      const t = setTimeout(() => setShown(null), 5000);
      return () => clearTimeout(t);
    }
  }, [last]);

  if (!shown) return null;
  return (
    <div className="animate-in fade-in slide-in-from-bottom-4 fixed bottom-11 left-1/2 z-40 flex -translate-x-1/2 items-center gap-2.5 rounded-lg border bg-popover px-4 py-2.5 text-xs shadow-xl">
      <CheckCircle2 className="size-4 text-emerald-500" />
      <span className="text-popover-foreground">
        预标注完成：<b className="tabular-nums">{shown.applied}</b>/{shown.total} 张图片获得标注，
        请逐张检查修正（低置信度区域已自动跳过）
      </span>
      <button
        className="ml-1 rounded p-0.5 text-muted-foreground hover:text-foreground"
        onClick={() => setShown(null)}
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

function App() {
  const init = useStore((s) => s.init);
  const importPaths = useStore((s) => s.importPaths);
  const setZoom = useStore((s) => s.setZoom);
  const requestFit = useStore((s) => s.requestFit);
  const zoom = useStore((s) => s.zoom);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);
  const dirty = useStore((s) => s.dirty);
  const flush = useStore((s) => s.flush);
  const history = useStore((s) => s.history);
  const current = useStore((s) => s.current);
  const images = useStore((s) => s.images);
  const annos = useStore((s) => s.annos);
  const dims = useStore((s) => s.dims);
  const selected = useStore((s) => s.selected);
  const propagating = useStore((s) => s.propagating);
  const propagateToAll = useStore((s) => s.propagateToAll);
  const setCurrent = useStore((s) => s.setCurrent);
  useStore((s) => s.historyTick);

  const [showExport, setShowExport] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    init()
      .catch((e) => console.error('初始化失败', e))
      .finally(() => setReady(true));
  }, [init]);

  useEffect(() => {
    const onBeforeUnload = () => void flush();
    window.addEventListener('beforeunload', onBeforeUnload);
    return () => window.removeEventListener('beforeunload', onBeforeUnload);
  }, [flush]);

  // 全局快捷键：撤销/重做 + ↑↓ 上下张
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.activeElement !== document.body) return;
      if (e.ctrlKey || e.metaKey) {
        const k = e.key.toLowerCase();
        if (k === 'z' && !e.shiftKey) {
          e.preventDefault();
          undo();
        } else if ((k === 'z' && e.shiftKey) || k === 'y') {
          e.preventDefault();
          redo();
        }
        return;
      }
      if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
        const s = useStore.getState();
        const idx = s.images.findIndex((i) => i.path === s.current);
        const next = idx + (e.key === 'ArrowDown' ? 1 : -1);
        if (next >= 0 && next < s.images.length) {
          e.preventDefault();
          s.setCurrent(s.images[next].path);
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [undo, redo, setCurrent]);

  const importFiles = async () => {
    const picked = await open({
      multiple: true,
      directory: false,
      filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp'] }],
    });
    if (picked) await importPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const importDir = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (picked) await importPaths([picked]);
  };

  if (!ready) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        加载中…
      </div>
    );
  }

  const dim = current ? dims[current] : undefined;
  const boxes = current ? (annos[current] ?? []) : [];
  const idx = current ? images.findIndex((i) => i.path === current) : -1;

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full flex-col bg-background">
        {/* ── 顶栏 = 标题栏（无边框窗口，可拖拽移动；双击最大化）── */}
        <header
          className="flex h-12 shrink-0 select-none items-center gap-1.5 border-b border-border/60 bg-card pl-3"
          data-tauri-drag-region
          onDoubleClick={(e) => {
            if (e.target === e.currentTarget) void getCurrentWindow().toggleMaximize();
          }}
        >
          <div className="mr-1 flex items-center gap-2.5" data-tauri-drag-region>
            <div className="flex size-6 items-center justify-center rounded-md bg-gradient-to-br from-orange-500 to-rose-500 text-[10px] font-bold text-white shadow-sm">
              标
            </div>
            <div className="flex flex-col leading-none" data-tauri-drag-region>
              <span className="text-[13px] font-semibold tracking-tight">dc_uitag</span>
              <span className="mt-0.5 text-[9px] tracking-widest text-muted-foreground/70 uppercase">
                annotator
              </span>
            </div>
          </div>

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-xs" onClick={importFiles}>
            <FileImage className="size-3.5" />
            图片
          </Button>
          <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-xs" onClick={importDir}>
            <FolderOpen className="size-3.5" />
            目录
          </Button>

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <ToolButton icon={<Undo2 className="size-3.5" />} label="撤销 (Ctrl+Z)" onClick={undo} disabled={!history.canUndo} />
          <ToolButton icon={<Redo2 className="size-3.5" />} label="重做 (Ctrl+Shift+Z)" onClick={redo} disabled={!history.canRedo} />

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <ToolButton icon={<ZoomOut className="size-3.5" />} label="缩小" onClick={() => setZoom(Math.max(0.1, zoom / 1.2))} />
          <button
            className="h-8 min-w-12 rounded-md px-1 text-xs tabular-nums text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={() => setZoom(1)}
            title="重置为 100%"
          >
            {Math.round(zoom * 100)}%
          </button>
          <ToolButton icon={<ZoomIn className="size-3.5" />} label="放大" onClick={() => setZoom(Math.min(5, zoom * 1.2))} />
          <ToolButton icon={<Maximize2 className="size-3.5" />} label="适应窗口" onClick={requestFit} />

          <div className="flex-1" />

          {dirty && (
            <span className="mr-1 flex items-center gap-1.5 text-[11px] text-muted-foreground/80">
              <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
              保存中
            </span>
          )}

          {/* 传播：当前图标注 → 全部其余图片 */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="secondary"
                size="sm"
                className="h-8 gap-1.5 text-xs"
                disabled={!current || boxes.length === 0 || propagating != null || images.length < 2}
                onClick={() => void propagateToAll()}
              >
                {propagating != null ? (
                  <span className="size-3.5 animate-spin rounded-full border-2 border-current border-t-transparent" />
                ) : (
                  <Sparkles className="size-3.5 text-primary" />
                )}
                {propagating != null ? `匹配中 ${propagating} 张…` : '自动预标注'}
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-64 text-left">
              以当前图的标注为模板，用模板匹配推算其余 {Math.max(images.length - 1, 0)} 张；
              低置信度自动跳过，结果需人工复核
            </TooltipContent>
          </Tooltip>

          <Button size="sm" className="h-8 gap-1.5 text-xs" onClick={() => setShowExport(true)}>
            <FileArchive className="size-3.5" />
            导出
          </Button>

          {/* 窗口控制：最小化 / 最大化 / 关闭（紧贴右上角） */}
          <WindowControls />
        </header>

        {/* ── 主体 ── */}
        <div className="flex min-h-0 flex-1">
          <ImageList />
          <main className="flex min-w-0 flex-1 flex-col">
            <TagPalette />
            <AnnoCanvas />
            {/* ── 状态栏 ── */}
            <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-border/60 bg-card px-3 text-[11px] text-muted-foreground">
              {current ? (
                <>
                  <span className="tabular-nums">
                    {idx + 1}/{images.length}
                  </span>
                  <span className="text-muted-foreground/40">·</span>
                  <span className="max-w-56 truncate">{images[idx]?.file_name}</span>
                  {dim && (
                    <>
                      <span className="text-muted-foreground/40">·</span>
                      <span className="tabular-nums">
                        {dim.width}×{dim.height}
                      </span>
                    </>
                  )}
                  <span className="text-muted-foreground/40">·</span>
                  <span className="tabular-nums">{boxes.length} 框</span>
                  {selected != null && boxes[selected] && (
                    <>
                      <span className="text-muted-foreground/40">·</span>
                      <span className="tabular-nums text-foreground/90">
                        {boxes[selected].tag} ({Math.round(boxes[selected].x)},
                        {Math.round(boxes[selected].y)}, {Math.round(boxes[selected].w)}×
                        {Math.round(boxes[selected].h)})
                      </span>
                    </>
                  )}
                </>
              ) : (
                <span>就绪</span>
              )}
              <div className="flex-1" />
              <span className="text-muted-foreground/50">↑↓ 切换 · Ctrl+滚轮 缩放 · 1-4 选标签</span>
            </footer>
          </main>
        </div>

        <RelabelMenu />
        <PropagateToast />
        <ExportDialog open={showExport} onOpenChange={setShowExport} />
      </div>
    </TooltipProvider>
  );
}

export default App;
