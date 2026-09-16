import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import * as api from './lib/tauri';
import { installShortcuts } from './lib/shortcuts';
import {
  IconArrowBackUp,
  IconArrowForwardUp,
  IconFileZip,
  IconFolderOpen,
  IconMagnet,
  IconMaximize,
  IconZoomIn,
  IconZoomOut,
} from '@tabler/icons-react';
import { useStore } from './store';
import { APP_NAME } from './lib/meta';
import { ImageList } from './components/ImageList';
import { TagPalette } from './components/TagPalette';
import { AnnoCanvas } from './components/AnnoCanvas';
import { ImageGrid } from './components/ImageGrid';
import { RelabelMenu } from './components/RelabelMenu';
import { ExportDialog } from './components/ExportDialog';
import { ShortcutsHelp } from './components/ShortcutsHelp';
import { AboutDialog } from './components/AboutDialog';
import { ThemeToggle } from './components/ThemeToggle';
import { WindowControls } from './components/WindowControls';
import { Button } from './components/ui/button';
import { Separator } from './components/ui/separator';
import { TooltipLayer } from './components/ui/tooltip-layer';

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
    <Button
      variant="ghost"
      size="icon"
      className="size-8 text-muted-foreground"
      onClick={onClick}
      disabled={disabled}
      data-tip={label}
    >
      {icon}
    </Button>
  );
}

/** 吸附开关：状态栏右侧磁铁图标按钮。 */
function SnapToggle() {
  const snapEnabled = useStore((s) => s.snapEnabled);
  const setSnapEnabled = useStore((s) => s.setSnapEnabled);
  return (
    <button
      className={
        'flex size-5 items-center justify-center rounded-md transition-colors ' +
        (snapEnabled
          ? 'bg-primary/15 text-primary'
          : 'text-muted-foreground/60 hover:bg-accent hover:text-foreground')
      }
      onClick={() => setSnapEnabled(!snapEnabled)}
      data-tip={snapEnabled ? '捕捉吸附：开（拖拽时吸附分割线，Ctrl 临时禁用，Alt 显示全部线）' : '捕捉吸附：关'}
    >
      <IconMagnet className="size-3.5" />
    </button>
  );
}

/** 吸附容差：数字文本点击变输入框（分割线判定的最小连续像素数）。 */
function SnapTolerance() {
  const tolerance = useStore((s) => s.snapTolerance);
  const setTolerance = useStore((s) => s.setSnapTolerance);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');

  const commit = () => {
    const v = Number(draft);
    if (Number.isFinite(v) && v > 0) setTolerance(v);
    setEditing(false);
  };

  if (editing) {
    return (
      <input
        autoFocus
        className="h-5 w-12 rounded-md bg-accent px-1 text-right text-xs tabular-nums text-foreground outline-none"
        value={draft}
        onChange={(e) => setDraft(e.target.value.replace(/[^\d]/g, ''))}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commit();
          if (e.key === 'Escape') setEditing(false);
        }}
      />
    );
  }
  return (
    <button
      className="min-w-8 rounded-md px-1 text-xs tabular-nums text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onClick={() => {
        setDraft(String(tolerance));
        setEditing(true);
      }}
      data-tip="吸附容差：判定分割线的最小连续像素数（点击修改）"
    >
      {tolerance}
    </button>
  );
}

function App() {  const init = useStore((s) => s.init);
  const openWorkspace = useStore((s) => s.openWorkspace);
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
  const dims = useStore((s) => s.dims);
  const tool = useStore((s) => s.tool);
  const selection = useStore((s) => s.selection);
  const multi = selection.length > 1;
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

  // 全局快捷键：定义在 lib/shortcuts 注册表，此处仅安装分发器
  useEffect(() => installShortcuts(), []);

  // 分割线异步识别：切图或容差变化时重新识别（Rust 竐缓存，不阻塞界面）
  const snapTolerance = useStore((s) => s.snapTolerance);
  const setSnapLines = useStore((s) => s.setSnapLines);
  useEffect(() => {
    if (!current) {
      setSnapLines([]);
      return;
    }
    let alive = true;
    api
      .detectLines(current, snapTolerance)
      .then((lines) => alive && setSnapLines(lines))
      .catch(() => alive && setSnapLines([]));
    return () => {
      alive = false;
    };
  }, [current, snapTolerance, setSnapLines]);

  const importDir = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (picked) await openWorkspace(picked);
  };

  if (!ready) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        加载中…
      </div>
    );
  }

  const dim = current ? dims[current] : undefined;
  const idx = current ? images.findIndex((i) => i.path === current) : -1;

  return (
    <>
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
            <img src="app-icon.png" alt={APP_NAME} className="size-6 rounded-md shadow-sm" />
            <span className="text-[13px] font-semibold tracking-tight" data-tauri-drag-region>
              {APP_NAME}
            </span>
          </div>

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-[13px]" onClick={importDir}>
            <IconFolderOpen className="size-3.5" />
            打开目录
          </Button>

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <ToolButton icon={<IconArrowBackUp className="size-3.5" />} label="撤销 (Ctrl+Z)" onClick={undo} disabled={!history.canUndo} />
          <ToolButton icon={<IconArrowForwardUp className="size-3.5" />} label="重做 (Ctrl+Y)" onClick={redo} disabled={!history.canRedo} />

          <Separator orientation="vertical" className="mx-1.5 !h-5" />

          <ToolButton icon={<IconZoomOut className="size-3.5" />} label="缩小" onClick={() => setZoom(Math.max(0.1, zoom / 1.2))} />
          <button
            className="h-8 min-w-12 rounded-md px-1 text-[13px] tabular-nums text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={() => setZoom(1)}
            data-tip="重置为 100%"
          >
            {Math.round(zoom * 100)}%
          </button>

          <ToolButton icon={<IconZoomIn className="size-3.5" />} label="放大" onClick={() => setZoom(Math.min(5, zoom * 1.2))} />
          <ToolButton icon={<IconMaximize className="size-3.5" />} label="适应窗口" onClick={requestFit} />

          <Separator orientation="vertical" className="mx-1.5 !h-5" />
          <ThemeToggle />
          <ShortcutsHelp />
          <AboutDialog />

          <div className="flex-1" />

          {dirty && (
            <span className="mr-1 flex items-center gap-1.5 text-xs text-muted-foreground/80">
              <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
              保存中
            </span>
          )}

          {/* 传播按钮在 TagPalette 右端 */}

          <Button
            size="sm"
            className="h-8 gap-1.5 text-[13px] shadow-sm transition-shadow hover:shadow-md"
            onClick={() => setShowExport(true)}
          >
            <IconFileZip className="size-3.5" />
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
            {multi ? <ImageGrid /> : <AnnoCanvas />}
            {/* ── 状态栏 ── */}
            <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-border/60 bg-card px-3 text-xs text-muted-foreground">
              {multi ? (
                <>
                  <span className="flex items-center gap-1.5 rounded bg-primary/15 px-1.5 py-0.5 text-[11px] font-medium text-primary">
                    <span className="size-1.5 rounded-full bg-primary" />
                    多选
                  </span>
                  <span className="tabular-nums">已选 {selection.length} 张</span>
                </>
              ) : current ? (
                <>
                  <span
                    className={
                      'flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[11px] font-medium transition-colors ' +
                      (tool === 'draw'
                        ? 'bg-primary/15 text-primary'
                        : 'bg-foreground/5 text-muted-foreground')
                    }
                  >
                    <span
                      className={
                        'size-1.5 rounded-full ' + (tool === 'draw' ? 'bg-primary' : 'bg-muted-foreground/60')
                      }
                    />
                    {tool === 'draw' ? '绘制' : '选择'}
                  </span>
                  <span className="tabular-nums">
                    {idx + 1}/{images.length}
                  </span>
                  <span className="text-muted-foreground/40">·</span>
                  <button
                    className="max-w-56 truncate underline decoration-dotted underline-offset-2 hover:text-foreground"
                    onClick={() => void api.revealPath(current)}
                    data-tip={current}
                    data-tip-hint="点击打开文件所在位置"
                    data-tip-class="max-w-md whitespace-normal font-mono break-all"
                    data-tip-side="top"
                  >
                    {images[idx]?.file_name}
                  </button>
                  {dim && (
                    <>
                      <span className="text-muted-foreground/40">·</span>
                      <span className="tabular-nums">
                        {dim.width}×{dim.height}
                      </span>
                    </>
                  )}
                </>
              ) : (
                <span>就绪</span>
              )}
              <div className="flex-1" />

              {/* 捕捉吸附：开关 + 容差（多选 grid 视图下隐藏） */}
              {!multi && (
                <div className="flex items-center gap-1.5">
                  <SnapToggle />
                  <SnapTolerance />
                </div>
              )}
            </footer>
          </main>
        </div>

        <RelabelMenu />
        <ExportDialog open={showExport} onOpenChange={setShowExport} />
        <TooltipLayer />
      </div>
    </>
  );
}

export default App;
