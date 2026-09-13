import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useStore } from './store';
import { ImageList } from './components/ImageList';
import { TagPalette } from './components/TagPalette';
import { AnnoCanvas } from './components/AnnoCanvas';
import { RelabelMenu } from './components/RelabelMenu';
import { ExportDialog } from './components/ExportDialog';

function App() {
  const init = useStore((s) => s.init);
  const importPaths = useStore((s) => s.importPaths);
  const setZoom = useStore((s) => s.setZoom);
  const zoom = useStore((s) => s.zoom);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);
  const dirty = useStore((s) => s.dirty);
  const flush = useStore((s) => s.flush);
  const historyTick = useStore((s) => s.historyTick);
  const history = useStore((s) => s.history);
  const [showExport, setShowExport] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    init()
      .catch((e) => console.error('初始化失败', e))
      .finally(() => setReady(true));
  }, [init]);

  // 关窗前冲刷自动保存
  useEffect(() => {
    const beforeUnload = () => {
      void flush();
    };
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, [flush]);

  // 全局快捷键：Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const key = e.key.toLowerCase();
      if (key === 'z' && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if ((key === 'z' && e.shiftKey) || key === 'y') {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [undo, redo]);

  const importFiles = async () => {
    const picked = await open({ multiple: true, directory: false });
    if (!picked) return;
    await importPaths(Array.isArray(picked) ? picked : [picked]);
  };
  const importDir = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (!picked) return;
    await importPaths([picked]);
  };

  if (!ready) {
    return <div className="flex h-screen items-center justify-center text-muted-foreground">加载中…</div>;
  }

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <span className="mr-2 text-sm font-semibold">dc_uitag</span>
        <button className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground" onClick={importFiles}>
          导入文件
        </button>
        <button className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-accent" onClick={importDir}>
          导入目录
        </button>
        <span className="mx-1 h-5 w-px bg-border" />
        <button
          className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40"
          onClick={undo}
          disabled={!history.canUndo}
          title="Ctrl+Z"
        >
          撤销
        </button>
        <button
          className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40"
          onClick={redo}
          disabled={!history.canRedo}
          title="Ctrl+Shift+Z"
        >
          重做
        </button>
        <span className="mx-1 h-5 w-px bg-border" />
        <span className="text-xs text-muted-foreground">{Math.round(zoom * 100)}%</span>
        <button className="rounded-md border border-border px-2 py-1.5 text-sm hover:bg-accent" onClick={() => setZoom(Math.min(5, zoom * 1.25))}>＋</button>
        <button className="rounded-md border border-border px-2 py-1.5 text-sm hover:bg-accent" onClick={() => setZoom(Math.max(0.1, zoom / 1.25))}>－</button>
        <button className="rounded-md border border-border px-2 py-1.5 text-sm hover:bg-accent" onClick={() => setZoom(1)}>1:1</button>
        <span className="flex-1" />
        {dirty && <span className="text-xs text-amber-500">未保存…</span>}
        <span className="hidden">{historyTick}</span>
        <button
          className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground"
          onClick={() => setShowExport(true)}
        >
          导出训练集
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <ImageList />
        <div className="flex min-w-0 flex-1 flex-col">
          <TagPalette />
          <AnnoCanvas />
        </div>
      </div>

      <RelabelMenu />
      {showExport && <ExportDialog onClose={() => setShowExport(false)} />}
    </div>
  );
}

export default App;
