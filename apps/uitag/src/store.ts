import { create } from 'zustand';
import type { AnnoBox, ImageEntry, SnapLine, TagsConfig } from './lib/types';
import { applyCmd, History, type Cmd } from './lib/commands';
import * as api from './lib/tauri';

/** 当前画布的实时拖拽状态（不入历史，pointerup 才落命令）。 */
export type DragState =
  | { kind: 'draw'; startX: number; startY: number; curX: number; curY: number }
  | { kind: 'move'; index: number; grabX: number; grabY: number; curX: number; curY: number; orig: AnnoBox }
  | {
      kind: 'resize';
      index: number;
      handle: Handle;
      /** 按下位置（图像坐标）：与手柄锚点的偏移要扣除，否则点一下手柄框就跳几像素 */
      grabX: number;
      grabY: number;
      curX: number;
      curY: number;
      orig: AnnoBox;
    }
  | null;

export type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

/** 画布工具态：draw = 十字光标拖拽画框；select = 常规光标选中/移动/缩放。 */
export type Tool = 'draw' | 'select';

interface UiTagStore {
  tags: TagsConfig;
  /** 当前工作目录（导入的图片目录），标注 json 保存于此 */
  workspaceDir: string | null;
  images: ImageEntry[];
  current: string | null;
  /** 图片自然尺寸：path → {width,height}（<img> onLoad 实测） */
  dims: Record<string, { width: number; height: number }>;
  annos: Record<string, AnnoBox[]>;
  activeTag: string;
  tool: Tool;
  selected: number | null;
  drag: DragState;
  zoom: number;
  /** 画布自由平移偏移（视口中心为原点，无限制；中键拖动） */
  pan: { x: number; y: number };
  /** 自增计数：请求画布重新按视口 fit（「适应窗口」按钮与切图都用它） */
  fitTick: number;
  history: History;
  historyTick: number; // 触发订阅重渲（History 本体非响应式）
  dirty: boolean; // 有未保存标注变更
  /** 预标注传播进行中（done/total 逐张推进） */
  propagating: { done: number; total: number } | null;
  /** 捕捉吸附开关 */
  snapEnabled: boolean;
  /** 吸附容差：分割线判定的最小连续像素数 */
  snapTolerance: number;
  /** 当前图片识别出的吸附线（异步识别完成后填充） */
  snapLines: SnapLine[];
  /** 图片多选：空 = 退化为单选（current）；长度 >1 时画布切换 grid 视图 */
  selection: string[];
  /** 当前图片锁定的框索引（会话级，双击切换；锁定框不可选中/拖拽） */
  locked: number[];

  setSnapEnabled: (v: boolean) => void;
  setSnapTolerance: (v: number) => void;
  setSnapLines: (lines: SnapLine[]) => void;
  /** 应用图片选择（单选 = 长度 1 或 0；多选 = 长度 >1，current 落在末项） */
  applySelection: (paths: string[]) => void;
  /** 清空给定图片的全部标注（逐图进撤销栈，Ctrl+Z 可还原） */
  clearImages: (paths: string[]) => void;
  toggleLock: (index: number) => void;

  init: () => Promise<void>;
  /** 打开（切换）工作目录：重载图片清单与该目录下的 annotations.json */
  openWorkspace: (dir: string) => Promise<void>;
  setCurrent: (path: string | null) => void;
  setDim: (path: string, width: number, height: number) => void;
  setActiveTag: (name: string) => void;
  setTool: (t: Tool) => void;
  /** 退出绘制状态：回到选择工具，并取消标签面板的选中项 */
  exitDraw: () => void;
  select: (index: number | null) => void;
  setDrag: (d: DragState) => void;
  setZoom: (z: number) => void;
  setPan: (p: { x: number; y: number }) => void;
  requestFit: () => void;

  commit: (cmd: Cmd) => void;
  undo: () => void;
  redo: () => void;
  deleteSelected: () => void;
  /** 清空当前图片全部标注（进撤销栈，Ctrl+Z 可还原） */
  clearCurrent: () => void;
  relabelBox: (index: number, tag: string) => void;
  /** 把当前图的标注传播到其余图片（NCC 模板匹配，预标注待人修）。
   *  overwrite=false 时只处理未标注的图片；返回 (获得标注张数, 处理总张数)。 */
  propagateToAll: (overwrite: boolean) => Promise<{ applied: number; total: number }>;

  flush: () => Promise<void>;
  exportAll: (dest: string) => Promise<string>;
}

const SAVE_DEBOUNCE_MS = 800;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/** 上一次工作目录的持久化 key（刷新/重启后自动恢复）。 */
const LAST_WORKDIR_KEY = 'uitag:last-workdir';
/** 上一次选中图片的持久化 key（恢复工作目录后定位到同一张）。 */
const LAST_CURRENT_KEY = 'uitag:last-current';

export const useStore = create<UiTagStore>((set, get) => ({
  tags: { version: 1, tags: [] },
  workspaceDir: null,
  images: [],
  current: null,
  dims: {},
  annos: {},
  activeTag: '',
  tool: 'draw',
  selected: null,
  drag: null,
  zoom: 1,
  pan: { x: 0, y: 0 },
  fitTick: 0,
  history: new History(),
  historyTick: 0,
  dirty: false,
  propagating: null,
  snapEnabled: true,
  snapTolerance: 100,
  snapLines: [],
  selection: [],
  locked: [],

  setSnapEnabled: (v) => set({ snapEnabled: v }),
  setSnapTolerance: (v) => set({ snapTolerance: Math.min(4096, Math.max(8, Math.round(v))) }),
  setSnapLines: (lines) => set({ snapLines: lines }),

  applySelection: (paths) =>
    set((s) => ({
      selection: paths,
      current: paths.length ? paths[paths.length - 1] : null,
      selected: null,
      drag: null,
      locked: [],
      fitTick: s.fitTick + 1,
    })),

  clearImages: (paths) => {
    const annos = get().annos;
    for (const p of paths) {
      const boxes = annos[p] ?? [];
      if (boxes.length > 0) get().commit({ kind: 'clear', path: p, boxes });
    }
  },

  toggleLock: (index) => {
    const s = get();
    const has = s.locked.includes(index);
    const locked = has ? s.locked.filter((i) => i !== index) : [...s.locked, index];
    if (!has && s.selected === index) set({ locked, selected: null });
    else set({ locked });
  },

  init: async () => {
    const tags = await api.loadTags();
    set({ tags, activeTag: tags.tags[0]?.name ?? '' });
    // 恢复上一次工作目录（目录可能已被移动/删除，失败则清掉记录正常启动）
    const last = localStorage.getItem(LAST_WORKDIR_KEY);
    if (last) {
      try {
        await get().openWorkspace(last);
        // 同步恢复上一次选中的图片
        const lastCur = localStorage.getItem(LAST_CURRENT_KEY);
        if (lastCur && get().images.some((i) => i.path === lastCur)) {
          get().setCurrent(lastCur);
        }
      } catch (e) {
        console.warn('恢复上一次工作目录失败', e);
        localStorage.removeItem(LAST_WORKDIR_KEY);
      }
    }
  },

  openWorkspace: async (dir) => {
    // 切换工作区前，先把旧目录未落盘的标注保存掉
    const prev = get();
    if (prev.workspaceDir && prev.dirty) await api.saveState(prev.workspaceDir, { annos: prev.annos });

    const { images, autosave } = await api.openWorkspace(dir);
    prev.history.clear();
    set({
      workspaceDir: dir,
      images,
      annos: autosave.annos,
      current: images[0]?.path ?? null,
      selected: null,
      drag: null,
      dirty: false,
      historyTick: prev.historyTick + 1,
      dims: {},
      snapLines: [],
      selection: [],
      locked: [],
    });
    localStorage.setItem(LAST_WORKDIR_KEY, dir);
  },

  setCurrent: (path) => {
    if (path) localStorage.setItem(LAST_CURRENT_KEY, path);
    set((s) => ({
      current: path,
      selection: [], // 单选导航 = 退出多选
      selected: null,
      drag: null,
      locked: [],
      fitTick: s.fitTick + 1,
    }));
  },
  setDim: (path, width, height) =>
    set((s) => ({ dims: { ...s.dims, [path]: { width, height } } })),
  setActiveTag: (name) => set({ activeTag: name, tool: 'draw' }),
  setTool: (t) => set({ tool: t }),
  exitDraw: () => set({ activeTag: '', tool: 'select' }),
  select: (index) => set({ selected: index }),
  setDrag: (d) => set({ drag: d }),
  setZoom: (z) => set({ zoom: z }),
  setPan: (p) => set({ pan: p }),
  requestFit: () => set((s) => ({ fitTick: s.fitTick + 1 })),

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

  clearCurrent: () => {
    const s = get();
    if (s.current == null) return;
    const boxes = s.annos[s.current] ?? [];
    if (boxes.length === 0) return;
    s.commit({ kind: 'clear', path: s.current, boxes });
    set({ selected: null });
  },

  relabelBox: (index, tag) => {
    const s = get();
    if (s.current == null) return;
    const box = (s.annos[s.current] ?? [])[index];
    if (!box || box.tag === tag) return;
    s.commit({ kind: 'relabel', path: s.current, index, before: box.tag, after: tag });
  },

  /** 把当前图的标注传播到其余图片（几何传播：窗口锚点检测 + 按结构映射）。
   *  overwrite=false 时只处理未标注的图片；逐张调用后端以推进进度。
   *  返回 (获得标注张数, 处理总张数)。 */
  propagateToAll: async (overwrite) => {
    const s = get();
    if (s.current == null) return { applied: 0, total: 0 };
    const boxes = s.annos[s.current] ?? [];
    if (boxes.length === 0) return { applied: 0, total: 0 };
    const targets = s.images
      .map((i) => i.path)
      .filter((p) => p !== s.current && (overwrite || (s.annos[p]?.length ?? 0) === 0));
    if (targets.length === 0) return { applied: 0, total: 0 };

    set({ propagating: { done: 0, total: targets.length } });
    try {
      const annos = { ...s.annos };
      let applied = 0;
      let done = 0;
      for (const t of targets) {
        const results = await api.propagateBoxes({
          src_path: s.current,
          boxes,
          targets: [t],
          min_confidence: 0.62,
          allow_rescale: true,
        });
        for (const r of results) {
          if (r.boxes.length > 0) applied++;
          annos[r.path] = r.boxes.map(({ confidence: _c, ...b }) => b);
        }
        done++;
        set({ propagating: { done, total: targets.length } });
      }
      // 传播是批量操作，不进撤销栈（73 张 × 4 框的命令回放没有意义）
      get().history.clear();
      set({ annos, historyTick: get().historyTick + 1, dirty: true });
      scheduleSave(get);
      return { applied, total: targets.length };
    } finally {
      set({ propagating: null });
    }
  },

  flush: async () => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    const s = get();
    if (!s.dirty || !s.workspaceDir) return;
    await api.saveState(s.workspaceDir, { annos: s.annos });
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
    if (!s.workspaceDir) return;
    api
      .saveState(s.workspaceDir, { annos: s.annos })
      .then(() => useStore.setState({ dirty: false }))
      .catch((e) => console.error('autosave 失败', e));
  }, SAVE_DEBOUNCE_MS);
}
