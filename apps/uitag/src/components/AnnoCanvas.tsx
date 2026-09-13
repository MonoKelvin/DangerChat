import { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ImageOff, Loader2, MousePointerSquareDashed } from 'lucide-react';
import { useStore, type Handle } from '../store';
import type { AnnoBox } from '../lib/types';

const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const MIN_BOX = 4;

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

/** 命中测试：最上层包含该点的框（手柄由自身元素命中，不经此处）。 */
function hitTest(boxes: AnnoBox[], px: number, py: number): number | null {
  for (let i = boxes.length - 1; i >= 0; i--) {
    const b = boxes[i];
    if (px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h) return i;
  }
  return null;
}

type LoadState = 'idle' | 'loading' | 'ready' | 'error';

export function AnnoCanvas() {
  const current = useStore((s) => s.current);
  const dims = useStore((s) => s.dims);
  const annos = useStore((s) => s.annos);
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const selected = useStore((s) => s.selected);
  const drag = useStore((s) => s.drag);
  const zoom = useStore((s) => s.zoom);
  const fitTick = useStore((s) => s.fitTick);
  const setDim = useStore((s) => s.setDim);
  const store = useStore;

  const svgRef = useRef<SVGSVGElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [load, setLoad] = useState<LoadState>('idle');

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
    const vp = viewportRef.current.getBoundingClientRect();
    const fit = Math.min((vp.width - 48) / dim.width, (vp.height - 48) / dim.height, 1);
    store.getState().setZoom(Math.max(0.1, Number(fit.toFixed(3))));
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
    const { x, y } = toImageXY(e);
    const hit = hitTest(boxes, x, y);
    const s = store.getState();
    if (hit != null) {
      s.select(hit);
      s.setDrag({ kind: 'move', index: hit, grabX: x, grabY: y, curX: 0, curY: 0, orig: boxes[hit] });
    } else {
      s.select(null);
      s.setDrag({ kind: 'draw', startX: x, startY: y, curX: x, curY: y });
    }
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const s = store.getState();
    const d = s.drag;
    if (!d) return;
    const { x, y } = toImageXY(e);
    if (d.kind === 'draw') s.setDrag({ ...d, curX: x, curY: y });
    else if (d.kind === 'move') s.setDrag({ ...d, curX: x - d.grabX, curY: y - d.grabY });
    else s.setDrag({ ...d, curX: x, curY: y });
  };

  const onPointerUp = () => {
    const s = store.getState();
    const d = s.drag;
    if (!d || !s.current) {
      s.setDrag(null);
      return;
    }
    const path = s.current;
    const min = MIN_BOX / zoom;
    if (d.kind === 'draw') {
      const w = Math.abs(d.curX - d.startX);
      const h = Math.abs(d.curY - d.startY);
      if (w >= min && h >= min && s.activeTag) {
        s.commit({
          kind: 'add',
          path,
          box: {
            tag: s.activeTag,
            x: Math.min(d.startX, d.curX),
            y: Math.min(d.startY, d.curY),
            w,
            h,
          },
        });
        s.select((s.annos[path]?.length ?? 1) - 1);
      }
    } else if (d.kind === 'move') {
      if (d.curX !== 0 || d.curY !== 0) {
        const after = { ...d.orig, x: d.orig.x + d.curX, y: d.orig.y + d.curY };
        s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
      }
    } else {
      const after = resizeFrom(d.orig, d.handle, d.curX, d.curY);
      const changed =
        after.x !== d.orig.x || after.y !== d.orig.y || after.w !== d.orig.w || after.h !== d.orig.h;
      if (changed && after.w >= min && after.h >= min) {
        s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
      }
    }
    s.setDrag(null);
  };

  const preview: AnnoBox | null = (() => {
    if (!drag) return null;
    if (drag.kind === 'draw') {
      return {
        tag: activeTag,
        x: Math.min(drag.startX, drag.curX),
        y: Math.min(drag.startY, drag.curY),
        w: Math.abs(drag.curX - drag.startX),
        h: Math.abs(drag.curY - drag.startY),
      };
    }
    if (drag.kind === 'move') {
      return { ...drag.orig, x: drag.orig.x + drag.curX, y: drag.orig.y + drag.curY };
    }
    return resizeFrom(drag.orig, drag.handle, drag.curX, drag.curY);
  })();

  const displayBoxes = boxes.map((b, i) =>
    drag && drag.kind !== 'draw' && drag.index === i ? (preview ?? b) : b,
  );

  // Delete 删除 / Esc 取消选中 / 数字键切标签
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.activeElement !== document.body) return;
      const s = store.getState();
      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (s.selected != null) {
          e.preventDefault();
          s.deleteSelected();
        }
      } else if (e.key === 'Escape') {
        s.select(null);
      } else if (/^[1-9]$/.test(e.key)) {
        const t = s.tags.tags[Number(e.key) - 1];
        if (t) s.setActiveTag(t.name);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [store]);

  const onWheel = useCallback((e: React.WheelEvent) => {
    if (!e.ctrlKey) return;
    const s = useStore.getState();
    const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
    s.setZoom(Math.min(5, Math.max(0.1, s.zoom * factor)));
  }, []);

  const empty = (icon: React.ReactNode, title: string, hint?: string) => (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-white/5 text-white/40">
        {icon}
      </div>
      <div>
        <p className="text-sm font-medium text-white/70">{title}</p>
        {hint && <p className="mt-1 text-xs text-white/40">{hint}</p>}
      </div>
    </div>
  );

  const hs = 4 / zoom;
  const sw = 1.5 / zoom;

  return (
    <div
      ref={viewportRef}
      className="relative flex-1 overflow-auto bg-canvas"
      style={{
        backgroundImage:
          'linear-gradient(var(--canvas-grid) 1px, transparent 1px), linear-gradient(90deg, var(--canvas-grid) 1px, transparent 1px)',
        backgroundSize: '24px 24px',
      }}
      onWheel={onWheel}
    >
      {!current && empty(<MousePointerSquareDashed className="size-6" />, '未选择图片', '从左侧列表选择一张截图开始标注')}
      {current && load === 'loading' && empty(<Loader2 className="size-6 animate-spin" />, '正在加载图片…')}
      {current && load === 'error' &&
        empty(<ImageOff className="size-6" />, '图片无法加载', '文件可能已被移动或删除')}

      {current && load === 'ready' && dim && (
        <div className="flex min-h-full min-w-full items-center justify-center p-6">
          <svg
            ref={svgRef}
            width={dim.width * zoom}
            height={dim.height * zoom}
            viewBox={`0 0 ${dim.width} ${dim.height}`}
            className="block shrink-0 cursor-crosshair rounded-sm ring-1 ring-white/10"
            style={{ touchAction: 'none', boxShadow: '0 8px 40px rgb(0 0 0 / 0.5)' }}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
          >
            <image href={convertFileSrc(current)} width={dim.width} height={dim.height} />

            {displayBoxes.map((b, i) => {
              const color = colorOf(b.tag);
              const isSel = selected === i;
              const label = `${b.tag} (${Math.round(b.x)},${Math.round(b.y)},${Math.round(b.w)},${Math.round(b.h)})`;
              return (
                <g
                  key={i}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    store.getState().select(i);
                    window.dispatchEvent(
                      new CustomEvent('uitag-relabel', {
                        detail: { index: i, x: e.clientX, y: e.clientY },
                      }),
                    );
                  }}
                >
                  <rect
                    x={b.x}
                    y={b.y}
                    width={b.w}
                    height={b.h}
                    fill={color}
                    fillOpacity={isSel ? 0.18 : 0.1}
                    stroke={color}
                    strokeWidth={isSel ? sw * 2 : sw}
                  />
                  {/* 标签牌：底色块 + 文字，避免压在深色截图上看不清 */}
                  <g transform={`translate(${b.x}, ${b.y})`}>
                    <rect
                      x={0}
                      y={-18 / zoom}
                      width={(label.length * 6.2 + 10) / zoom}
                      height={16 / zoom}
                      rx={3 / zoom}
                      fill={color}
                    />
                    <text
                      x={5 / zoom}
                      y={-6 / zoom}
                      fontSize={10 / zoom}
                      fill="#fff"
                      className="pointer-events-none select-none font-medium"
                    >
                      {label}
                    </text>
                  </g>
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
                          fill="#fff"
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

            {drag?.kind === 'draw' && preview && (
              <rect
                x={preview.x}
                y={preview.y}
                width={preview.w}
                height={preview.h}
                fill={colorOf(preview.tag)}
                fillOpacity={0.1}
                stroke={colorOf(preview.tag)}
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
