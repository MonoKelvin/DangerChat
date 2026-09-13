import { create } from 'zustand';
import type { AnnoBox, ImageEntry, TagsConfig } from './lib/types';
import { applyCmd, History, type Cmd } from './lib/commands';
import * as api from './lib/tauri';

/** 当前画布的实时拖拽状态（不入历史，pointerup 才落命令）。 */
export type DragState =
  | { kind: 'draw'; startX: number; startY: number; curX: number; curY: number }
  | { kind: 'move'; index: number; grabX: number; grabY: number; curX: number; curY: number; orig: AnnoBox }
  | { kind: 'resize'; index: number; handle: Handle; curX: number; curY: number; orig: AnnoBox }
  | null;

export type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

interface UiTagStore {
  tags: TagsConfig;
  images: ImageEntry[];
  current: string | null;
  /** 图片自然尺寸：path → {width,height}（<img> onLoad 实测） */
  dims: Record<string, { width: number; height: number }>;
  annos: Record<string, AnnoBox[]>;
  activeTag: string;
  selected: number | null;
  drag: DragState;
  zoom: number;
  history: History;
  historyTick: number; // 触发订阅重渲（History 本体非响应式）
  dirty: boolean; // 有未保存标注变更

  init: () => Promise<void>;
  importPaths: (paths: string[]) => Promise<void>;
  setCurrent: (path: string | null) => void;
  setDim: (path: string, width: number, height: number) => void;
  setActiveTag: (name: string) => void;
  select: (index: number | null) => void;
  setDrag: (d: DragState) => void;
  setZoom: (z: number) => void;

  commit: (cmd: Cmd) => void;
  undo: () => void;
  redo: () => void;
  deleteSelected: () => void;
  relabelBox: (index: number, tag: string) => void;
  previewDragBox: () => AnnoBox | null;

  flush: () => Promise<void>;
  exportAll: (dest: string) => Promise<string>;
}

const SAVE_DEBOUNCE_MS = 800;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

export const useStore = create<UiTagStore>((set, get) => ({
  tags: { version: 1, tags: [] },
  images: [],
  current: null,
  dims: {},
  annos: {},
  activeTag: '',
  selected: null,
  drag: null,
  zoom: 1,
  history: new History(),
  historyTick: 0,
  dirty: false,

  init: async () => {
    const [tags, state] = await Promise.all([api.loadTags(), api.loadState()]);
    const annos = state?.annos ?? {};
    const firstTag = tags.tags[0]?.name ?? '';
    set({ tags, annos, activeTag: firstTag });
  },

  importPaths: async (paths) => {
    const images = await api.listImages(paths);
    set((s) => {
      const seen = new Set(s.images.map((i) => i.path));
      const fresh = images.filter((i) => !seen.has(i.path));
      return { images: [...s.images, ...fresh] };
    });
    const { current } = get();
    if (!current && images.length > 0) set({ current: images[0].path });
  },

  setCurrent: (path) => set({ current: path, selected: null, drag: null }),
  setDim: (path, width, height) =>
    set((s) => ({ dims: { ...s.dims, [path]: { width, height } } })),
  setActiveTag: (name) => set({ activeTag: name }),
  select: (index) => set({ selected: index }),
  setDrag: (d) => set({ drag: d }),
  setZoom: (z) => set({ zoom: z }),

  commit: (cmd) => {
    const s = get();
    s.history.push(cmd);
    set({
      annos: applyCmd(s.annos, cmd, 'do'),
      historyTick: s.historyTick + 1,
      dirty: true,
    });
    scheduleSave(get);
  },

  undo: () => {
    const s = get();
    const cmd = s.history.popUndo();
    if (!cmd) return;
    set({
      annos: applyCmd(s.annos, cmd, 'undo'),
      historyTick: s.historyTick + 1,
      dirty: true,
    });
    scheduleSave(get);
  },

  redo: () => {
    const s = get();
    const cmd = s.history.popRedo();
    if (!cmd) return;
    set({
      annos: applyCmd(s.annos, cmd, 'do'),
      historyTick: s.historyTick + 1,
      dirty: true,
    });
    scheduleSave(get);
  },

  deleteSelected: () => {
    const s = get();
    if (s.current == null || s.selected == null) return;
    const list = s.annos[s.current] ?? [];
    const box = list[s.selected];
    if (!box) return;
    s.commit({ kind: 'delete', path: s.current, index: s.selected, box });
    set({ selected: null });
  },

  relabelBox: (index, tag) => {
    const s = get();
    if (s.current == null) return;
    const box = (s.annos[s.current] ?? [])[index];
    if (!box || box.tag === tag) return;
    s.commit({ kind: 'relabel', path: s.current, index, before: box.tag, after: tag });
  },

  /** 拖拽进行中的预览框（绘框 / 移动 / 缩放三种统一为「当前应显示的框」）。 */
  previewDragBox: () => {
    const s = get();
    if (s.current == null || !s.drag) return null;
    const d = s.drag;
    if (d.kind === 'draw') {
      return normRect(d.startX, d.startY, d.curX, d.curY, s.activeTag);
    }
    if (d.kind === 'move') {
      return { ...d.orig, x: d.orig.x + d.curX, y: d.orig.y + d.curY };
    }
    return null;
  },

  flush: async () => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    const s = get();
    if (!s.dirty) return;
    await api.saveState({ annos: s.annos });
    set({ dirty: false });
  },

  exportAll: async (dest) => {
    const s = get();
    const images = s.images.map((i) => ({
      src: i.path,
      width: s.dims[i.path]?.width ?? 0,
      height: s.dims[i.path]?.height ?? 0,
    }));
    // key 由 path 换成文件名主干（与后端 stem_of 对齐）
    const annos: Record<string, AnnoBox[]> = {};
    for (const [path, boxes] of Object.entries(s.annos)) {
      const stem = stemOf(path);
      annos[stem] = boxes;
    }
    await s.flush();
    return api.exportZip({ images, annos, tags: s.tags, dest });
  },
}));

function normRect(
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  tag: string,
): AnnoBox {
  return {
    tag,
    x: Math.min(x1, x2),
    y: Math.min(y1, y2),
    w: Math.abs(x2 - x1),
    h: Math.abs(y2 - y1),
  };
}

export function stemOf(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? path;
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(0, dot) : base;
}

function scheduleSave(get: () => UiTagStore) {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const s = get();
    api
      .saveState({ annos: s.annos })
      .then(() => useStore.setState({ dirty: false }))
      .catch((e) => console.error('autosave 失败', e));
  }, SAVE_DEBOUNCE_MS);
}
