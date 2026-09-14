import { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { IconLoader2, IconPhotoOff, IconSquareDashed } from '@tabler/icons-react';
import { useStore, type DragState, type Handle } from '../store';
import type { AnnoBox, SnapLine } from '../lib/types';
import { cn } from '@/lib/utils';

const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const MIN_BOX = 4;

/** 把 hex 颜色压暗到 f 比例（锚点芯色：与框同色系的深色，不用白色）。 */
function darken(hex: string, f: number): string {
  const n = parseInt(hex.slice(1), 16);
  const r = Math.round(((n >> 16) & 255) * f);
  const g = Math.round(((n >> 8) & 255) * f);
  const b = Math.round((n & 255) * f);
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`;
}

const CURSOR: Record<Handle, string> = {
  nw: 'nwse-resize', n: 'ns-resize', ne: 'nesw-resize', e: 'ew-resize',
  se: 'nwse-resize', s: 'ns-resize', sw: 'nesw-resize', w: 'ew-resize',
};

/** 对侧锚点：resize 时固定不动的一角。 */
const OPPOSITE: Record<Handle, Handle> = {
  nw: 'se', n: 's', ne: 'sw', e: 'w', se: 'nw', s: 'n', sw: 'ne', w: 'e',
};

function anchorOf(box: AnnoBox, h: Handle): { x: number; y: number } {
  const { x, y, w, h: bh } = box;
  const cx = x + w / 2;
  const cy = y + bh / 2;
  const map: Record<Handle, { x: number; y: number }> = {
    nw: { x, y },
    n: { x: cx, y },
    ne: { x: x + w, y },
    e: { x: x + w, y: cy },
    se: { x: x + w, y: y + bh },
    s: { x: cx, y: y + bh },
    sw: { x, y: y + bh },
    w: { x, y: cy },
  };
  return map[h];
}

/** 以对侧锚点为不动点，从拖拽点重算矩形（允许越轴拖成反向框）。 */
function resizeFrom(orig: AnnoBox, handle: Handle, px: number, py: number): AnnoBox {
  const a = anchorOf(orig, OPPOSITE[handle]);
  const horiz = handle === 'e' || handle === 'w';
  const vert = handle === 'n' || handle === 's';
  // 边手柄只改一个轴
  const x1 = vert ? orig.x : Math.min(a.x, px);
  const w = vert ? orig.w : Math.abs(px - a.x);
  const y1 = horiz ? orig.y : Math.min(a.y, py);
  const h = horiz ? orig.h : Math.abs(py - a.y);
  return { ...orig, x: x1, y: y1, w, h };
}

/** 把框夹回图片内部：移动/缩放/绘制均不得越出图片边界。 */
function clampBox(box: AnnoBox, dim: { width: number; height: number }): AnnoBox {
  const w = Math.min(box.w, dim.width);
  const h = Math.min(box.h, dim.height);
  return {
    ...box,
    w,
    h,
    x: Math.min(Math.max(box.x, 0), dim.width - w),
    y: Math.min(Math.max(box.y, 0), dim.height - h),
  };
}

/** ── 捕捉吸附 ── */

/** 找距 pos 最近、且与框边垂直方向跨度有交集的吸附线（|delta| ≤ thr）。 */
function snapEdge(
  pos: number,
  lo: number,
  hi: number,
  orient: 'h' | 'v',
  lines: SnapLine[],
  thr: number,
): { delta: number; line: SnapLine } | null {
  let best: { delta: number; line: SnapLine } | null = null;
  for (const l of lines) {
    if (l.orient !== orient) continue;
    // 线的跨度需与框边范围有足够交集（≥4 图像像素），远处的短线不吸附
    const ov = Math.min(hi, l.end) - Math.max(lo, l.start);
    if (ov < 4) continue;
    const delta = l.pos - pos;
    if (Math.abs(delta) <= thr && (!best || Math.abs(delta) < Math.abs(best.delta))) {
      best = { delta, line: l };
    }
  }
  return best;
}

/** 按应用字体实测文字宽度（屏幕像素）——标签牌自适应文字长度，避免右侧大片空白。 */
let measureCtx: CanvasRenderingContext2D | null = null;
function textWidth(text: string, fontSize: number): number {
  if (!measureCtx) measureCtx = document.createElement('canvas').getContext('2d');
  if (!measureCtx) return text.length * fontSize * 0.6;
  measureCtx.font =
    `500 ${fontSize}px "Segoe UI Variable Text", "Segoe UI", system-ui, ` +
    `"Microsoft YaHei UI", "Microsoft YaHei", sans-serif`;
  return measureCtx.measureText(text).width;
}

/** 标签牌底板路径：只在指定角加圆角（贴合框边的角保持直角）。 */function platePath(
  w: number,
  h: number,
  r: number,
  tl = false,
  tr = false,
  br = false,
  bl = false,
): string {
  const parts = [
    `M${tl ? r : 0},0`,
    `H${w - (tr ? r : 0)}`,
    tr ? `Q${w},0 ${w},${r}` : '',
    `V${h - (br ? r : 0)}`,
    br ? `Q${w},${h} ${w - r},${h}` : '',
    `H${bl ? r : 0}`,
    bl ? `Q0,${h} 0,${h - r}` : '',
    `V${tl ? r : 0}`,
    tl ? `Q0,0 ${r},0` : '',
    'Z',
  ];
  return parts.filter(Boolean).join(' ');
}

/** 独立点吸附：绘制起点用。x/y 分别吸附到最近且跨度覆盖该点的线（|delta| ≤ thr）。 */function snapPoint(
  x: number,
  y: number,
  lines: SnapLine[],
  thr: number,
): { x: number; y: number } {
  let sx = x;
  let sy = y;
  for (const l of lines) {
    if (l.orient === 'v' && Math.abs(l.pos - x) <= thr && y >= l.start - 4 && y <= l.end + 4) {
      sx = l.pos;
    } else if (l.orient === 'h' && Math.abs(l.pos - y) <= thr && x >= l.start - 4 && x <= l.end + 4) {
      sy = l.pos;
    }
  }
  return { x: sx, y: sy };
}

/**
 * 对框应用吸附。moveMode = 整体移动（保持宽高，按最近的边位移吸附）；
 * 否则（绘制/缩放）各边独立吸附（可改尺寸）。
 * 返回吸附后的框 + 参考线（用于画虚线指示）；apply=false 时只算参考线不改框。
 */function snapBox(
  box: AnnoBox,
  lines: SnapLine[],
  thr: number,
  apply: boolean,
  moveMode: boolean,
): { box: AnnoBox; guides: SnapLine[] } {
  if (moveMode) {
    // 左右两边取更近的垂直线 → 位移 dx；上下同理
    const dxs = [snapEdge(box.x, box.y, box.y + box.h, 'v', lines, thr),
                 snapEdge(box.x + box.w, box.y, box.y + box.h, 'v', lines, thr)]
      .filter(Boolean)
      .sort((a, b) => Math.abs(a!.delta) - Math.abs(b!.delta));
    const dys = [snapEdge(box.y, box.x, box.x + box.w, 'h', lines, thr),
                 snapEdge(box.y + box.h, box.x, box.x + box.w, 'h', lines, thr)]
      .filter(Boolean)
      .sort((a, b) => Math.abs(a!.delta) - Math.abs(b!.delta));
    const dx = dxs[0]?.delta ?? 0;
    const dy = dys[0]?.delta ?? 0;
    const guides = [dxs[0]?.line, dys[0]?.line].filter(Boolean) as SnapLine[];
    return {
      box: apply ? { ...box, x: box.x + dx, y: box.y + dy } : box,
      guides,
    };
  }
  // 各边独立吸附；上下（或左右）吸附到同一条线时只保留更近的边，避免压扁
  let top = snapEdge(box.y, box.x, box.x + box.w, 'h', lines, thr);
  let bottom = snapEdge(box.y + box.h, box.x, box.x + box.w, 'h', lines, thr);
  if (top && bottom && top.line === bottom.line) {
    if (Math.abs(top.delta) <= Math.abs(bottom.delta)) bottom = null;
    else top = null;
  }
  let left = snapEdge(box.x, box.y, box.y + box.h, 'v', lines, thr);
  let right = snapEdge(box.x + box.w, box.y, box.y + box.h, 'v', lines, thr);
  if (left && right && left.line === right.line) {
    if (Math.abs(left.delta) <= Math.abs(right.delta)) right = null;
    else left = null;
  }
  const guides = [top, bottom, left, right].map((s) => s?.line).filter(Boolean) as SnapLine[];
  const next: AnnoBox = apply
    ? {
        ...box,
        y: top ? box.y + top.delta : box.y,
        h: top || bottom ? box.h + (top ? -top.delta : 0) + (bottom ? bottom.delta : 0) : box.h,
        x: left ? box.x + left.delta : box.x,
        w: left || right ? box.w + (left ? -left.delta : 0) + (right ? right.delta : 0) : box.w,
      }
    : box;
  return { box: next, guides };
}

/** 命中测试：选中的框优先（它渲染在最上层），其余从最上层往下找；
 *  锁定的框（locked）不可命中，方便选择其内部的其他框。 */
function hitTest(
  boxes: AnnoBox[],
  px: number,
  py: number,
  topIndex: number | null,
  locked: ReadonlySet<number>,
): number | null {
  const inside = (b: AnnoBox) => px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h;
  if (topIndex != null && !locked.has(topIndex) && inside(boxes[topIndex])) return topIndex;
  for (let i = boxes.length - 1; i >= 0; i--) {
    if (i !== topIndex && !locked.has(i) && inside(boxes[i])) return i;
  }
  return null;
}

/** 实时拖拽预览时把光标点夹回图片内（仅 draw/resize：curX/curY 是图像坐标）。
 *  move 的 curX/curY 是位移增量（可为负），不能钳制——由 clampBox 钳最终框。 */
function clampDrag(
  d: Extract<NonNullable<DragState>, { kind: 'draw' | 'resize' }>,
  dim: { width: number; height: number },
): NonNullable<DragState> {
  const cx = Math.min(Math.max(d.curX, 0), dim.width);
  const cy = Math.min(Math.max(d.curY, 0), dim.height);
  if (cx === d.curX && cy === d.curY) return d;
  return { ...d, curX: cx, curY: cy };
}

type LoadState = 'idle' | 'loading' | 'ready' | 'error';

export function AnnoCanvas() {
  const current = useStore((s) => s.current);
  const dims = useStore((s) => s.dims);
  const annos = useStore((s) => s.annos);
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const tool = useStore((s) => s.tool);
  const selected = useStore((s) => s.selected);
  const drag = useStore((s) => s.drag);
  const zoom = useStore((s) => s.zoom);
  const panOffset = useStore((s) => s.pan);
  const fitTick = useStore((s) => s.fitTick);
  const setDim = useStore((s) => s.setDim);
  const snapLines = useStore((s) => s.snapLines);
  const snapEnabled = useStore((s) => s.snapEnabled);
  const store = useStore;

  const svgRef = useRef<SVGSVGElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [load, setLoad] = useState<LoadState>('idle');
  /** 拖拽期间按住 Ctrl → 不吸附（参考线仍显示） */
  const ctrlRef = useRef(false);
  /** 按住 Alt → 显示全部识别线 */
  const [showAllLines, setShowAllLines] = useState(false);
  /** 绘制模式下的悬停位置（用于吸附十字标记） */
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  const locked = useStore((s) => s.locked);
  const toggleLock = useStore((s) => s.toggleLock);
  /** 刚解锁的框索引：短暂保留以播放锁图标退场动画 */
  const [lockExiting, setLockExiting] = useState<Set<number>>(new Set());
  const prevLockedRef = useRef<number[]>([]);

  useEffect(() => {
    const removed = prevLockedRef.current.filter((i) => !locked.includes(i));
    prevLockedRef.current = locked;
    if (removed.length === 0) return;
    setLockExiting(new Set(removed));
    const t = setTimeout(() => setLockExiting(new Set()), 320);
    return () => clearTimeout(t);
  }, [locked]);

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Alt') {
        e.preventDefault();
        setShowAllLines(true);
      } else if (e.key === 'Control') {
        ctrlRef.current = true;
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === 'Alt') setShowAllLines(false);
      else if (e.key === 'Control') ctrlRef.current = false;
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  }, []);

  const dim = current ? dims[current] : undefined;
  const boxes = current ? (annos[current] ?? []) : [];
  const colorOf = (tag: string) => tags.find((t) => t.name === tag)?.color ?? '#94a3b8';

  /**
   * 图片自然尺寸测量：SVG <image> 不会回报尺寸，必须先用 Image() 预加载。
   * 这一步缺失会让画布永远停在「加载中」——务必保持。
   */
  useEffect(() => {
    if (!current) {
      setLoad('idle');
      return;
    }
    if (dims[current]) {
      setLoad('ready');
      return;
    }
    let alive = true;
    setLoad('loading');
    const img = new Image();
    img.onload = () => {
      if (!alive) return;
      setDim(current, img.naturalWidth, img.naturalHeight);
      setLoad('ready');
    };
    img.onerror = () => alive && setLoad('error');
    img.src = convertFileSrc(current);
    return () => {
      alive = false;
    };
  }, [current, dims, setDim]);

  /** 按视口自适应缩放：切图或点「适应窗口」时触发（fitTick 自增）。 */
  useEffect(() => {
    if (load !== 'ready' || !dim || !viewportRef.current) return;
    stopZoomAnim(); // 适应窗口按视口整体重算，终止进行中的缩放动画
    zoomAnchor.current = null;
    const vp = viewportRef.current.getBoundingClientRect();
    const fit = Math.min((vp.width - 48) / dim.width, (vp.height - 48) / dim.height, 1);
    const s = store.getState();
    s.setPan({ x: 0, y: 0 });
    s.setZoom(Math.max(0.1, Number(fit.toFixed(3))));
  }, [fitTick, load, dim, store]);

  const toImageXY = useCallback(
    (e: { clientX: number; clientY: number }): { x: number; y: number } => {
      const rect = svgRef.current?.getBoundingClientRect();
      if (!rect) return { x: 0, y: 0 };
      return { x: (e.clientX - rect.left) / zoom, y: (e.clientY - rect.top) / zoom };
    },
    [zoom],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    if (!current || e.button !== 0) return;
    ctrlRef.current = e.ctrlKey;
    const { x, y } = toImageXY(e);
    const s = store.getState();
    const hit = hitTest(boxes, x, y, s.selected, new Set(locked));
    if (hit != null) {
      // 命中已有框：无论什么工具态都是选中 + 拖动
      s.select(hit);
      s.setDrag({ kind: 'move', index: hit, grabX: x, grabY: y, curX: 0, curY: 0, orig: boxes[hit] });
    } else if (tool === 'draw') {
      s.select(null);
      // 绘制起点吸附到识别线（Ctrl 按住不吸附）
      const p = snapEnabled && !e.ctrlKey ? snapPoint(x, y, snapLines, 8 / zoom) : { x, y };
      s.setDrag({ kind: 'draw', startX: p.x, startY: p.y, curX: p.x, curY: p.y });
    } else {
      s.select(null);
      return; // 选择模式下点空白：仅取消选中，不捕获指针
    }
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    ctrlRef.current = e.ctrlKey;
    const s = store.getState();
    const d = s.drag;
    const { x, y } = toImageXY(e);
    if (!d) {
      // 绘制模式下记录悬停位置（吸附十字标记用）
      setHover(s.tool === 'draw' ? { x, y } : null);
      return;
    }
    setHover(null);
    if (d.kind === 'draw') s.setDrag({ ...d, curX: x, curY: y });
    else if (d.kind === 'move') s.setDrag({ ...d, curX: x - d.grabX, curY: y - d.grabY });
    else s.setDrag({ ...d, curX: x, curY: y });
  };

  const onPointerUp = () => {
    const s = store.getState();
    const d0 = s.drag;
    if (!d0 || !s.current) {
      s.setDrag(null);
      return;
    }
    const path = s.current;
    const min = MIN_BOX / zoom;
    const d = dim && d0.kind !== 'move' ? clampDrag(d0, dim) : d0;
    if (d.kind === 'draw') {
      const w = Math.abs(d.curX - d.startX);
      const h = Math.abs(d.curY - d.startY);
      if (w >= min && h >= min && s.activeTag) {
        const raw = {
          tag: s.activeTag,
          x: Math.min(d.startX, d.curX),
          y: Math.min(d.startY, d.curY),
          w,
          h,
        };
        s.commit({ kind: 'add', path, box: applySnap(clampBox(raw, dim!), false).box });
        // 绘制完成后不选中新框：不出现锚点，可直接继续画下一个
      } else {
        // 绘制模式下点了一下空白：退出绘制（含取消标签选中）
        s.exitDraw();
      }
    } else if (d.kind === 'move') {
      if (d.curX !== 0 || d.curY !== 0) {
        const after = applySnap(
          clampBox({ ...d.orig, x: d.orig.x + d.curX, y: d.orig.y + d.curY }, dim!),
          true,
        ).box;
        s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
      }
    } else {
      const after = applySnap(clampBox(resizeFrom(d.orig, d.handle, d.curX, d.curY), dim!), false).box;
      const changed =
        after.x !== d.orig.x || after.y !== d.orig.y || after.w !== d.orig.w || after.h !== d.orig.h;
      if (changed && after.w >= min && after.h >= min) {
        s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
      }
    }
    s.setDrag(null);
  };

  /** 捕捉吸附：8 屏幕像素内吸附到识别线；Ctrl 按住或开关关闭时只显示参考线 */
  const applySnap = (box: AnnoBox, moveMode: boolean): { box: AnnoBox; guides: SnapLine[] } => {
    if (snapLines.length === 0) return { box, guides: [] };
    const apply = snapEnabled && !ctrlRef.current;
    return snapBox(box, snapLines, 8 / zoom, apply, moveMode);
  };

  const preview: AnnoBox | null = (() => {
    if (!drag || !dim) return null;
    const d = drag.kind === 'move' ? drag : clampDrag(drag, dim);
    if (d.kind === 'draw') {
      return clampBox(
        {
          tag: activeTag,
          x: Math.min(d.startX, d.curX),
          y: Math.min(d.startY, d.curY),
          w: Math.abs(d.curX - d.startX),
          h: Math.abs(d.curY - d.startY),
        },
        dim,
      );
    }
    if (d.kind === 'move') {
      return clampBox({ ...d.orig, x: d.orig.x + d.curX, y: d.orig.y + d.curY }, dim);
    }
    return clampBox(resizeFrom(d.orig, d.handle, d.curX, d.curY), dim);
  })();

  const snapRes = preview ? applySnap(preview, drag?.kind === 'move') : null;
  const snapped = snapRes?.box ?? preview;
  const snapGuides = snapRes?.guides ?? [];

  // 绘制模式悬停点吸附：附近有可吸附线时显示十字标记，点击从标记处起画
  const drawSnap = (() => {
    if (!hover || tool !== 'draw' || !snapEnabled || ctrlRef.current || snapLines.length === 0) {
      return null;
    }
    const p = snapPoint(hover.x, hover.y, snapLines, 8 / zoom);
    return p.x === hover.x && p.y === hover.y ? null : p;
  })();

  const displayBoxes = boxes.map((b, i) =>
    drag && drag.kind !== 'draw' && drag.index === i ? (snapped ?? b) : b,
  );

  // 渲染顺序：选中的框最后画（SVG 文档序即层序），保证锚点不被其他框盖住、
  // 拖动/缩放手柄始终可点；索引不变，只调整绘制顺序。
  const renderOrder = displayBoxes.map((_, i) => i);
  if (selected != null) {
    const si = renderOrder.indexOf(selected);
    if (si >= 0) {
      renderOrder.splice(si, 1);
      renderOrder.push(selected);
    }
  }

  // Delete / Esc / 数字键等键盘命令统一在 lib/shortcuts 注册表分发（App 安装）

  // 滚轮缩放：锚定光标下的图像点；中键拖动平移。React onWheel 是 passive，
  // 必须手动挂 non-passive 监听才能 preventDefault。
  const zoomAnchor = useRef<{ clientX: number; clientY: number; imgX: number; imgY: number } | null>(null);
  const pan = useRef<{ startX: number; startY: number; lastX: number; lastY: number } | null>(null);

  // 流畅缩放（参考 照片 UWP）：滚轮只更新目标倍率，rAF 指数逼近（快→缓渐变），
  // 连续滚动目标持续累积；每帧校正滚动保持光标下的图像点不动。
  const zoomTargetRef = useRef(1);
  const zoomRafRef = useRef<number | null>(null);

  const stopZoomAnim = () => {
    if (zoomRafRef.current != null) {
      cancelAnimationFrame(zoomRafRef.current);
      zoomRafRef.current = null;
    }
  };

  /** 缩放 k0→k1：按锚点同步校正平移，使光标下的图像点保持不动。
   *  svg 中心 = 视口中心 + pan，故 pan' = pan + (img - dim/2)·(k0 − k1)。 */
  const applyZoomAnchored = (
    s: ReturnType<typeof useStore.getState>,
    k0: number,
    k1: number,
  ) => {
    const a = zoomAnchor.current;
    const d = s.current ? s.dims[s.current] : undefined;
    if (a && d) {
      s.setPan({
        x: s.pan.x + (a.imgX - d.width / 2) * (k0 - k1),
        y: s.pan.y + (a.imgY - d.height / 2) * (k0 - k1),
      });
    }
    s.setZoom(k1);
  };

  const animateZoom = (target: number) => {
    zoomTargetRef.current = target;
    if (zoomRafRef.current != null) return; // 已在跑，只改目标
    const step = () => {
      const s = useStore.getState();
      const cur = s.zoom;
      const t = zoomTargetRef.current;
      const next = cur + (t - cur) * 0.22;
      if (Math.abs(t - next) < 0.0015) {
        zoomRafRef.current = null;
        applyZoomAnchored(s, cur, t);
        // 动画结束即锚点使命完成（延迟到本帧提交后清理）
        requestAnimationFrame(() => (zoomAnchor.current = null));
        return;
      }
      applyZoomAnchored(s, cur, next);
      zoomRafRef.current = requestAnimationFrame(step);
    };
    zoomRafRef.current = requestAnimationFrame(step);
  };

  useEffect(() => stopZoomAnim, []); // 卸载清理

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const onWheel = (e: WheelEvent) => {
      const svg = svgRef.current;
      if (!svg) return;
      e.preventDefault();
      const s = useStore.getState();
      const factor = Math.min(1.3, Math.max(1 / 1.3, Math.exp(-e.deltaY * 0.0016)));
      // 动画进行中在目标倍率上累积，否则从当前倍率起算
      const base = zoomRafRef.current != null ? zoomTargetRef.current : s.zoom;
      const next = Math.min(5, Math.max(0.1, base * factor));
      if (next === base) return;
      const r = svg.getBoundingClientRect();
      zoomAnchor.current = {
        clientX: e.clientX,
        clientY: e.clientY,
        imgX: (e.clientX - r.left) / s.zoom,
        imgY: (e.clientY - r.top) / s.zoom,
      };
      animateZoom(next);
    };

    const onAuxClick = (e: MouseEvent) => {
      if (e.button === 1) e.preventDefault(); // 阻止中键自动滚动
    };

    vp.addEventListener('wheel', onWheel, { passive: false });
    vp.addEventListener('auxclick', onAuxClick);
    return () => {
      vp.removeEventListener('wheel', onWheel);
      vp.removeEventListener('auxclick', onAuxClick);
    };
  }, []);

  const onViewportPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 1) return;
    e.preventDefault();
    pan.current = { startX: e.clientX, startY: e.clientY, lastX: e.clientX, lastY: e.clientY };
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  };

  const onViewportPointerMove = (e: React.PointerEvent) => {
    if (!pan.current) return;
    const s = store.getState();
    // 鼠标拖多少图片跟多少（同向）
    s.setPan({
      x: s.pan.x + (e.clientX - pan.current.lastX),
      y: s.pan.y + (e.clientY - pan.current.lastY),
    });
    pan.current = { ...pan.current, lastX: e.clientX, lastY: e.clientY };
  };

  const onViewportPointerUp = (e: React.PointerEvent) => {
    if (!pan.current) return;
    const { startX, startY } = pan.current;
    pan.current = null;
    // 位移很小 = 单击中键：流畅缩放到 100%（锚定点击位置，与滚轮缩放同动画）
    const moved = Math.hypot(e.clientX - startX, e.clientY - startY);
    if (moved > 4) return;
    const svg = svgRef.current;
    const s = store.getState();
    if (!svg || !s.current) return;
    const r = svg.getBoundingClientRect();
    zoomAnchor.current = {
      clientX: e.clientX,
      clientY: e.clientY,
      imgX: (e.clientX - r.left) / s.zoom,
      imgY: (e.clientY - r.top) / s.zoom,
    };
    animateZoom(1);
  };

  const empty = (icon: React.ReactNode, title: string, hint?: string) => (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-foreground/5 text-muted-foreground">
        {icon}
      </div>
      <div>
        <p className="text-sm font-medium text-foreground/70">{title}</p>
        {hint && <p className="mt-1 text-xs text-muted-foreground">{hint}</p>}
      </div>
    </div>
  );

  const hs = 4 / zoom;
  const sw = 1.5 / zoom;

  return (
    <div
      ref={viewportRef}
      className="relative flex-1 overflow-hidden bg-canvas"
      style={{
        backgroundImage:
          'linear-gradient(var(--canvas-grid) 1px, transparent 1px), linear-gradient(90deg, var(--canvas-grid) 1px, transparent 1px)',
        backgroundSize: '24px 24px',
      }}
      onPointerDown={(e) => {
        const s = store.getState();
        // 点图片外的背景（画布空白区域）：取消选中；绘制模式下同时退出绘制
        if (e.button === 0 && svgRef.current && !svgRef.current.contains(e.target as Node)) {
          s.select(null);
          if (s.tool === 'draw') s.exitDraw();
        }
        onViewportPointerDown(e);
      }}
      onPointerMove={onViewportPointerMove}
      onPointerUp={onViewportPointerUp}
      onPointerCancel={onViewportPointerUp}
    >
      {!current && empty(<IconSquareDashed className="size-6" />, '未选择图片', '从左侧列表选择一张截图开始标注')}
      {current && load === 'loading' && empty(<IconLoader2 className="size-6 animate-spin" />, '正在加载图片…')}
      {current && load === 'error' &&
        empty(<IconPhotoOff className="size-6" />, '图片无法加载', '文件可能已被移动或删除')}

      {current && load === 'ready' && dim && (
        /* 自由平移：视口大小容器 + translate 偏移，无滚动限制（任何缩放都能拖动） */
        <div
          className="flex h-full w-full items-center justify-center"
          style={{ transform: `translate(${panOffset.x}px, ${panOffset.y}px)` }}
        >
          <svg
            ref={svgRef}
            width={dim.width * zoom}
            height={dim.height * zoom}
            viewBox={`0 0 ${dim.width} ${dim.height}`}
            className={cn(
              'block shrink-0 rounded-sm ring-1 ring-border transition-shadow',
              tool === 'draw' && !drag ? 'cursor-crosshair' : 'cursor-default',
            )}
            style={{
              touchAction: 'none',
              overflow: 'visible',
              boxShadow: '0 4px 16px rgb(0 0 0 / 0.25)',
              // 拖拽期间明确光标：移动 = move 图标，绘制 = 十字（覆盖 class 的 crosshair/default）
              cursor: drag ? (drag.kind === 'draw' ? 'crosshair' : 'move') : undefined,
            }}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
            onPointerLeave={() => setHover(null)}
            onDoubleClick={(e) => {
              // 双击 = 锁定/解锁最上层命中的框（含已锁定的，用于解锁）。
              // 必须在 svg 层做命中：pointerdown 的 setPointerCapture 会把
              // click/dblclick 重定向到 svg，框元素上的 onDoubleClick 收不到。
              if (!current || e.button !== 0) return;
              const { x, y } = toImageXY(e);
              for (let i = boxes.length - 1; i >= 0; i--) {
                const b = boxes[i];
                if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) {
                  toggleLock(i);
                  return;
                }
              }
            }}
          >
            <image href={convertFileSrc(current)} width={dim.width} height={dim.height} />

            {renderOrder.map((i) => {
              const b = displayBoxes[i];
              const color = colorOf(b.tag);
              const isSel = selected === i;
              const isLocked = locked.includes(i);
              // 编号 = 创建顺序（数组序），从 1 起
              const label = `${i + 1}: ${b.tag}`;
              // 标签牌：横向优先；太窄改竖向（沿框左边）。内部显示需完全容纳
              // （横向：宽够长 + 高够厚；竖向：高够长 + 宽够厚），放不下才移到
              // 框外（横向 → 框上方，竖向 → 框左侧）。圆角只出现在不贴合框边的角。
              const lh = 18 / zoom; // 牌厚度
              const lw = (textWidth(label, 11) + 4 + 12) / zoom; // 牌长度 = 实测文字宽 + 左 4 右 12 间距
              const lr = 4 / zoom; // 圆角半径
              const fitsHoriz = b.w >= lw; // 横向文字长度放得下
              const vertical = !fitsHoriz; // 太窄 → 竖向
              const labelInside = vertical
                ? b.w >= lh && b.h >= lw // 竖向：厚度 + 长度都容得下
                : b.h >= lh; // 横向：厚度容得下
              return (
                <g
                  key={i}
                  style={{ cursor: isLocked ? 'default' : 'move' }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    window.dispatchEvent(
                      new CustomEvent('uitag-relabel', {
                        detail: { index: i, x: e.clientX, y: e.clientY, locked: isLocked },
                      }),
                    );
                    if (!isLocked) store.getState().select(i);
                  }}
                >
                  <rect
                    x={b.x}
                    y={b.y}
                    width={b.w}
                    height={b.h}
                    fill={color}
                    fillOpacity={isSel ? 0.12 : 0.06}
                    stroke={color}
                    strokeWidth={isSel ? sw * 2 : sw}
                  />
                  {/* 标签牌：半透明可见底下内容；圆角只在不贴合框边的角上 */}
                  {!vertical ? (
                    /* 横向：内部 = 贴框顶/左边，圆右下角；外部(框上方) = 底边贴框顶，圆上两角 */
                    <g
                      transform={`translate(${b.x}, ${b.y + (labelInside ? 0 : -lh - 3 / zoom)})`}
                    >
                      <path
                        d={labelInside ? platePath(lw, lh, lr, false, false, true) : platePath(lw, lh, lr, true, true)}
                        fill={color}
                        fillOpacity={0.82}
                      />
                      <text
                        x={4 / zoom}
                        y={12.5 / zoom}
                        fontSize={11 / zoom}
                        fill="#fff"
                        className="pointer-events-none select-none font-medium"
                      >
                        {label}
                      </text>
                    </g>
                  ) : (
                    /* 竖向：内部 = 贴框顶/左边，圆右下角；外部(框左侧) = 右边贴框左边，圆左两角 */
                    <g
                      transform={`translate(${b.x + (labelInside ? 0 : -lh - 3 / zoom)}, ${b.y})`}
                    >
                      <path
                        d={labelInside ? platePath(lh, lw, lr, false, false, true) : platePath(lh, lw, lr, true, false, false, true)}
                        fill={color}
                        fillOpacity={0.82}
                      />
                      <text
                        /* 竖排：rotate(-90) 后文字沿 -y 向上延伸，锚点放牌底部起排 */
                        transform={`translate(${12.5 / zoom}, ${lw - 6 / zoom}) rotate(-90)`}
                        fontSize={11 / zoom}
                        fill="#fff"
                        className="pointer-events-none select-none font-medium"
                      >
                        {label}
                      </text>
                    </g>
                  )}
                  {(isLocked || lockExiting.has(i)) && (
                    /* 锁定标记：框中心的大号锁（同框颜色、半透明），弹簧动画进出 */
                    <g
                      transform={`translate(${b.x + b.w / 2}, ${b.y + b.h / 2}) scale(${1 / zoom})`}
                      className="pointer-events-none"
                    >
                      <g className={cn('lock-spring', isLocked ? 'lock-spring-in' : 'lock-spring-out')}>
                        <path
                          d="M -9 -6 v-5 a9 9 0 0 1 18 0 v5"
                          fill="none"
                          stroke={color}
                          strokeWidth={4}
                          strokeLinecap="round"
                        />
                        <rect x={-13} y={-6} width={26} height={20} rx={4} fill={color} fillOpacity={0.6} stroke={color} strokeWidth={3} />
                      </g>
                    </g>
                  )}
                  {isSel &&
                    HANDLES.map((h) => {
                      const p = anchorOf(b, h);
                      return (
                        <rect
                          key={h}
                          x={p.x - hs}
                          y={p.y - hs}
                          width={hs * 2}
                          height={hs * 2}
                          fill={darken(color, 0.45)}
                          stroke={color}
                          strokeWidth={sw}
                          style={{ cursor: CURSOR[h] }}
                          onPointerDown={(e) => {
                            e.stopPropagation();
                            const s = store.getState();
                            const pt = toImageXY(e);
                            s.select(i);
                            s.setDrag({
                              kind: 'resize',
                              index: i,
                              handle: h,
                              curX: pt.x,
                              curY: pt.y,
                              orig: boxes[i],
                            });
                            (e.currentTarget as Element).setPointerCapture(e.pointerId);
                          }}
                        />
                      );
                    })}
                </g>
              );
            })}

            {/* 捕捉吸附参考线：拖拽时显示临近可吸附线（主色虚线）；
                Alt 按住显示全部识别线（弱化白虚线）。画成贯穿画面的无限延长线。 */}
            {(snapGuides.length > 0 || showAllLines) && (
              <g className="pointer-events-none">
                {(showAllLines ? snapLines : snapGuides).map((l, i) => {
                  const ext = 500 / zoom; // 视觉上“无限长”，超出画面两端
                  return l.orient === 'h' ? (
                    <line
                      key={i}
                      x1={-ext}
                      y1={l.pos}
                      x2={dim!.width + ext}
                      y2={l.pos}
                      stroke={showAllLines ? 'var(--color-muted-foreground)' : 'var(--color-primary)'}
                      opacity={showAllLines ? 0.45 : undefined}
                      strokeWidth={sw}
                      strokeDasharray={`${6 / zoom} ${4 / zoom}`}
                    />
                  ) : (
                    <line
                      key={i}
                      x1={l.pos}
                      y1={-ext}
                      x2={l.pos}
                      y2={dim!.height + ext}
                      stroke={showAllLines ? 'var(--color-muted-foreground)' : 'var(--color-primary)'}
                      opacity={showAllLines ? 0.45 : undefined}
                      strokeWidth={sw}
                      strokeDasharray={`${6 / zoom} ${4 / zoom}`}
                    />
                  );
                })}
              </g>
            )}

            {/* 绘制起点的吸附圆点标记（当前标签颜色） */}
            {drawSnap && (
              <g className="pointer-events-none">
                <circle
                  cx={drawSnap.x}
                  cy={drawSnap.y}
                  r={4.5 / zoom}
                  fill={colorOf(activeTag)}
                  stroke="#fff"
                  strokeWidth={1.5 / zoom}
                />
              </g>
            )}

            {drag?.kind === 'draw' && snapped && (
              <rect
                x={snapped.x}
                y={snapped.y}
                width={snapped.w}
                height={snapped.h}
                fill={colorOf(snapped.tag)}
                fillOpacity={0.08}
                stroke={colorOf(snapped.tag)}
                strokeDasharray={`${5 / zoom} ${3 / zoom}`}
                strokeWidth={sw}
              />
            )}
          </svg>
        </div>
      )}
    </div>
  );
}
