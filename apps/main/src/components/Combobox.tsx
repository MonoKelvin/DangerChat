import { useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown } from 'lucide-react';
import { cn } from '../lib/utils';

export interface ComboOption {
  value: string;
  label: string;
  /** 副说明（右侧灰字） */
  hint?: string;
}

interface ComboboxProps {
  value: string;
  options: ComboOption[];
  onChange: (v: string) => void;
  disabled?: boolean;
  /** 触发器宽度 class，默认 w-40；面板宽度独立按内容自适应，不受此项影响 */
  className?: string;
  /** 触发器宽度自适应内容（按最长选项实测），用于防护应用等长标签场景 */
  fitContent?: boolean;
}

/** 面板最大宽度（px）：超过则换行/截断，不无限制撑开。 */
const PANEL_MAX_W = 420;
/** 面板最小宽度（px）：选项很短时也保证可点面积。 */
const PANEL_MIN_W = 160;
/** 视口左右留白（px）。 */
const VIEWPORT_GAP = 12;
/** 单项内容的水平内边距（px）：面板 p-1.5(6) ×2 + 项 px-3(12) ×2。 */
const ITEM_INSET = 36;
/** 单项高度（px）：内容行高 20 + py-2(8) ×2。 */
const ITEM_H = 36;
/** 面板垂直内边距合计（px）：p-1.5(6) ×2。 */
const PANEL_PAD_Y = 12;
/** 滚动条预留宽度（px）：选项多时面板出现纵向滚动条，统一预留避免文字被压或宽度跳动。 */
const SCROLLBAR_W = 10;

/** 触发器宽度与面板宽度解耦：面板按**最长选项**测量宽度，避免 w-24/w-32 这类窄触发器
 *  把「微信（Weixin.exe）」「dc-layout-wechat」等长标签截断。
 *
 *  实现是离屏 DOM 实测而非字数估算——中英文混排（CJK 逐字宽、拉丁字母窄）下按字符数
 *  推算误差很大，而测量成本只在展开时一次。 */
function measureContentWidth(options: ComboOption[]): number {
  const probe = document.createElement('div');
  probe.style.cssText = 'position:absolute;visibility:hidden;white-space:nowrap;top:-9999px;left:-9999px;';
  document.body.appendChild(probe);
  let widest = 0;
  for (const o of options) {
    probe.innerHTML = '';
    const row = document.createElement('span');
    row.style.cssText = 'display:inline-flex;align-items:center;gap:8px;font-size:14px;';
    const label = document.createElement('span');
    label.textContent = o.label;
    row.appendChild(label);
    if (o.hint) {
      const hint = document.createElement('span');
      hint.style.cssText = 'font-size:12px;';
      hint.textContent = o.hint;
      row.appendChild(hint);
    }
    // 勾选占位：size-4(16) + gap 8，所有项都预留，保证与渲染一致
    row.insertAdjacentHTML('afterbegin', '<span style="width:16px;flex:0 0 16px;"></span>');
    probe.appendChild(row);
    widest = Math.max(widest, row.getBoundingClientRect().width);
  }
  probe.remove();
  return widest + ITEM_INSET + SCROLLBAR_W;
}

/** 触发器最小宽度（px）：fitContent 时短标签也保持可点面积。 */
const TRIGGER_MIN_W = 128;
/** 触发器最大宽度（px）：fitContent 时长标签不超过此上限，超出 truncate。 */
const TRIGGER_MAX_W = 280;
/** 触发器内部不可压缩部分（px）：px-3.5(14)×2 + gap-2(8) + chevron size-4(16)。 */
const TRIGGER_INSET = 52;

/** 触发器宽度按**当前选中项**实测（不含 hint，因为触发器不渲染 hint）。 */
function measureTriggerWidth(label: string): number {
  const probe = document.createElement('div');
  probe.style.cssText =
    'position:absolute;visibility:hidden;white-space:nowrap;top:-9999px;left:-9999px;font-size:14px;';
  probe.textContent = label;
  document.body.appendChild(probe);
  const w = probe.getBoundingClientRect().width;
  probe.remove();
  return Math.min(TRIGGER_MAX_W, Math.max(TRIGGER_MIN_W, w + TRIGGER_INSET));
}

/** 下拉选择：不允许输入；面板高斯模糊浮层，选中项打勾。
 *
 *  面板宽度按内容自适应（不跟随触发器），仅右对齐触发器并在视口内做钳制与左右翻转。 */
export function Combobox({
  value,
  options,
  onChange,
  disabled,
  className,
  fitContent,
}: ComboboxProps) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [contentW, setContentW] = useState(0);
  // 测量只在「展开」这一刻做一次。依赖项刻意不含 options：调用点普遍在渲染期新建
  // options 字面量（map/展开），引用每次渲染都变，若纳入依赖会导致 effect 反复
  // setState → 重渲染 → 再触发，形成无限循环。展开后再改选项（如搜索过滤）不重测，
  // 属可接受的近似——面板宽度有 MIN/MAX 钳制，最坏也只是宽一点。
  const optionsRef = useRef(options);
  optionsRef.current = options;

  useLayoutEffect(() => {
    if (!open || !btnRef.current) return;
    setRect(btnRef.current.getBoundingClientRect());
    setContentW(measureContentWidth(optionsRef.current));
  }, [open]);

  const selected = options.find((o) => o.value === value);
  const triggerW = fitContent ? measureTriggerWidth(selected?.label ?? value) : undefined;

  // 面板宽度：内容实测 → 钳到 [MIN, MAX]；再受视口限制（两侧各留 VIEWPORT_GAP）
  const panelW = Math.max(
    PANEL_MIN_W,
    Math.min(contentW || PANEL_MIN_W, PANEL_MAX_W, window.innerWidth - VIEWPORT_GAP * 2),
  );
  // 优先右对齐触发器；面板比触发器宽时向左伸展，左缘越界则贴视口左缘。
  // 设置页控件都在行右侧，右对齐让下拉框看起来是从触发器正下方长出来的。
  const left = rect
    ? Math.max(VIEWPORT_GAP, Math.min(rect.right - panelW, window.innerWidth - panelW - VIEWPORT_GAP))
    : 0;
  // 单项高度 = 内容行高(20) + py-2(8)×2；面板上下内边距 p-1.5(6)×2
  const estHeight = Math.min(options.length * ITEM_H + PANEL_PAD_Y, window.innerHeight - VIEWPORT_GAP * 2);
  const top = rect
    ? Math.min(rect.bottom + 6, Math.max(VIEWPORT_GAP, window.innerHeight - VIEWPORT_GAP - estHeight))
    : 0;

  // 视口内可用的最大高度：面板最多占到触发器下方的空间，不足再向上要。
  // top 已保证上边界 >= VIEWPORT_GAP，故 maxHeight 取其到视口底的距离。
  const maxH = rect
    ? Math.max(ITEM_H + PANEL_PAD_Y, window.innerHeight - top - VIEWPORT_GAP)
    : window.innerHeight - VIEWPORT_GAP * 2;

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        style={triggerW ? { width: triggerW } : undefined}
        className={cn(
          'flex h-9 items-center justify-between gap-2 rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150',
          'hover:bg-[var(--active-overlay)] focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
          'disabled:pointer-events-none disabled:opacity-40',
          open && 'bg-[var(--group-bg)] shadow-[inset_0_0_0_1.5px_var(--brand)]',
          // 统一默认宽度：面板已按内容自适应，触发器不必再为「装下长选项」而各自加宽，
          // 只保证当前选中项可读即可（超长仍 truncate）。fitContent 时改用实测内联宽度。
          triggerW ? 'shrink-0' : (className ?? 'w-40'),
        )}
      >
        <span className="min-w-0 flex-1 truncate text-left">{selected?.label ?? value}</span>
        <ChevronDown
          className={cn('size-4 shrink-0 text-[var(--text-tertiary)] transition-transform duration-200', open && 'rotate-180')}
          strokeWidth={2}
        />
      </button>

      {open &&
        rect &&
        createPortal(
          <>
            {/* 点击外部关闭（z 高于 Modal 的 z-[100]，否则面板被对话框遮住） */}
            <div className="fixed inset-0 z-[120]" onPointerDown={() => setOpen(false)} />
            <div
              role="listbox"
              className="animate-in fade-in-0 zoom-in-95 fixed z-[125] overflow-y-auto overflow-x-hidden rounded-xl border border-[var(--glass-border)] bg-[var(--popover-blur)] p-1.5 shadow-[var(--shadow-lg)] backdrop-blur-2xl"
              style={{ left, top, width: panelW, maxHeight: maxH }}
            >
              {options.map((o) => (
                <button
                  key={o.value}
                  role="option"
                  aria-selected={o.value === value}
                  onClick={() => {
                    onChange(o.value);
                    setOpen(false);
                  }}
                  className={cn(
                    'flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors',
                    'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
                  )}
                >
                  <Check
                    className={cn(
                      'size-4 shrink-0 text-[var(--brand)] transition-opacity',
                      o.value === value ? 'opacity-100' : 'opacity-0',
                    )}
                    strokeWidth={2.5}
                  />
                  <span className="flex-1">{o.label}</span>
                  {o.hint && (
                    <span className="shrink-0 text-xs text-[var(--text-tertiary)]">{o.hint}</span>
                  )}
                </button>
              ))}
            </div>
          </>,
          document.body,
        )}
    </>
  );
}
