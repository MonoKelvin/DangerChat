import { useCallback, useEffect, useRef } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useStore, type Handle } from '../store';
import type { AnnoBox } from '../lib/types';

const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
const MIN_BOX = 4;

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
  return {
    ...orig,
    x: Math.min(a.x, px),
    y: Math.min(a.y, py),
    w: Math.abs(px - a.x),
    h: Math.abs(py - a.y),
  };
}

/** 命中测试：返回 {kind:'box', index} 或 null（手柄由自身元素命中，不经此处）。 */
function hitTest(
  boxes: AnnoBox[],
  px: number,
  py: number,
): { kind: 'box'; index: number } | null {
  for (let i = boxes.length - 1; i >= 0; i--) {
    const b = boxes[i];
    if (px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h) {
      return { kind: 'box', index: i };
    }
  }
  return null;
}

export function AnnoCanvas() {
  const current = useStore((s) => s.current);
  const dims = useStore((s) => s.dims);
  const annos = useStore((s) => s.annos);
  const tags = useStore((s) => s.tags.tags);
  const activeTag = useStore((s) => s.activeTag);
  const selected = useStore((s) => s.selected);
  const drag = useStore((s) => s.drag);
  const zoom = useStore((s) => s.zoom);
  const store = useStore;

  const svgRef = useRef<SVGSVGElement>(null);

  const dim = current ? dims[current] : undefined;
  const boxes = current ? (annos[current] ?? []) : [];
  const colorOf = (tag: string) => tags.find((t) => t.name === tag)?.color ?? '#888888';

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

    if (hit?.kind === 'box') {
      store.getState().select(hit.index);
      store.getState().setDrag({
        kind: 'move',
        index: hit.index,
        grabX: x,
        grabY: y,
        curX: 0,
        curY: 0,
        orig: boxes[hit.index],
      });
    } else {
      store.getState().select(null);
      store.getState().setDrag({ kind: 'draw', startX: x, startY: y, curX: x, curY: y });
    }
    (e.target as Element).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = store.getState().drag;
    if (!d) return;
    const { x, y } = toImageXY(e);
    if (d.kind === 'draw') {
      store.getState().setDrag({ ...d, curX: x, curY: y });
    } else if (d.kind === 'move') {
      store.getState().setDrag({ ...d, curX: x - d.grabX, curY: y - d.grabY });
    } else if (d.kind === 'resize') {
      store.getState().setDrag({ ...d, curX: x, curY: y });
    }
  };

  const onPointerUp = () => {
    const s = store.getState();
    const d = s.drag;
    if (!d || !s.current) {
      s.setDrag(null);
      return;
    }
    const path = s.current;
    if (d.kind === 'draw') {
      const w = Math.abs(d.curX - d.startX);
      const h = Math.abs(d.curY - d.startY);
      if (w >= MIN_BOX / zoom && h >= MIN_BOX / zoom) {
        const box: AnnoBox = {
          tag: s.activeTag,
          x: Math.min(d.startX, d.curX),
          y: Math.min(d.startY, d.curY),
          w,
          h,
        };
        s.commit({ kind: 'add', path, box });
      }
    } else if (d.kind === 'move' && (d.curX !== 0 || d.curY !== 0)) {
      const after = { ...d.orig, x: d.orig.x + d.curX, y: d.orig.y + d.curY };
      if (after.x !== d.orig.x || after.y !== d.orig.y) {
        s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
      }
    } else if (d.kind === 'resize') {
      const after = resizeFrom(d.orig, d.handle, d.curX, d.curY);
      if (after.x !== d.orig.x || after.y !== d.orig.y || after.w !== d.orig.w || after.h !== d.orig.h) {
        if (after.w >= MIN_BOX / zoom && after.h >= MIN_BOX / zoom) {
          s.commit({ kind: 'transform', path, index: d.index, before: d.orig, after });
        }
      }
    }
    s.setDrag(null);
  };

  // 拖拽中的实时预览框
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

  // 显示列表：拖拽替换目标框
  const displayBoxes = boxes.map((b, i) => {
    if (drag && drag.kind !== 'draw' && drag.index === i) return preview ?? b;
    return b;
  });

  // 快捷键：Delete / 1-4 切标签（画布挂全局 keydown）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = store.getState();
      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (s.selected != null && document.activeElement === document.body) {
          e.preventDefault();
          s.deleteSelected();
        }
      } else if (/^[1-9]$/.test(e.key) && document.activeElement === document.body) {
        const t = s.tags.tags[Number(e.key) - 1];
        if (t) s.setActiveTag(t.name);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [store]);

  if (!current || !dim) {
    return (
      <div className="flex flex-1 items-center justify-center text-muted-foreground">
        {current ? '正在加载图片…' : '导入图片后选择一张开始标注'}
      </div>
    );
  }

  const hs = 4 / zoom; // 手柄边长（视觉恒定）
  const sw = 1.5 / zoom; // 描边宽（视觉恒定）

  return (
    <div className="flex-1 overflow-auto bg-neutral-200 p-4">
      <svg
        ref={svgRef}
        width={dim.width * zoom}
        height={dim.height * zoom}
        viewBox={`0 0 ${dim.width} ${dim.height}`}
        className="mx-auto block cursor-crosshair bg-white shadow-md"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onWheel={onWheel}
        style={{ touchAction: 'none' }}
      >
        <image href={convertFileSrc(current)} width={dim.width} height={dim.height} />

        {displayBoxes.map((b, i) => {
          const color = colorOf(b.tag);
          const isSel = selected === i;
          return (
            <g key={i} onContextMenu={(e) => onContextMenu(e, i)}>
              <rect
                x={b.x}
                y={b.y}
                width={b.w}
                height={b.h}
                fill={color}
                fillOpacity={0.12}
                stroke={color}
                strokeWidth={isSel ? sw * 1.6 : sw}
              />
              <text
                x={b.x}
                y={b.y - 4 / zoom}
                fontSize={12 / zoom}
                fill={color}
                className="select-none font-medium"
              >
                {`${b.tag} (${Math.round(b.x)},${Math.round(b.y)},${Math.round(b.w)},${Math.round(b.h)})`}
              </text>
              {isSel &&
                HANDLES.map((h) => {
                  const p = anchorOf(b, h);
                  return (
                    <rect
                      key={h}
                      data-handle={h}
                      x={p.x - hs}
                      y={p.y - hs}
                      width={hs * 2}
                      height={hs * 2}
                      fill="#fff"
                      stroke={color}
                      strokeWidth={sw}
                      className="cursor-pointer"
                      onPointerDown={(e) => {
                        e.stopPropagation();
                        const s = store.getState();
                        const p = toImageXY(e);
                        s.select(i);
                        s.setDrag({ kind: 'resize', index: i, handle: h, curX: p.x, curY: p.y, orig: boxes[i] });
                        (e.target as Element).setPointerCapture(e.pointerId);
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
            fill="none"
            stroke={colorOf(preview.tag)}
            strokeDasharray={`${4 / zoom} ${3 / zoom}`}
            strokeWidth={sw}
          />
        )}
      </svg>
    </div>
  );
}

function onWheel(e: React.WheelEvent) {
  if (!e.ctrlKey) return;
  e.preventDefault();
  const s = useStore.getState();
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  s.setZoom(Math.min(5, Math.max(0.1, s.zoom * factor)));
}

function onContextMenu(e: React.MouseEvent, _index: number) {
  e.preventDefault();
  // 右键改标签：先选中该框，弹出标签菜单
  useStore.getState().select(_index);
  // 简易菜单：用 window.confirm 逐个询问太糙——交给 TagContextMenu
  window.dispatchEvent(
    new CustomEvent('uitag-relabel', { detail: { index: _index } }),
  );
}
