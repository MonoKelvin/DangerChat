import type { ReactNode } from 'react';
import { useStore } from '../store';

/**
 * 快捷键注册表（唯一定义处）：
 * - 按键分发（App 全局 keydown）按 match/run 执行
 * - 快捷键面板（ShortcutsHelp）按 group/desc/keys 渲染
 * 命令定义一次、注册一次，使用方直接获取，避免多处重复维护。
 */
export type GroupId = 'draw' | 'edit' | 'nav';

export interface ShortcutItem {
  id: string;
  group: GroupId;
  desc: string;
  keys: ReactNode;
  /** 缺省 = 纯说明条目（鼠标交互），不参与按键分发 */
  match?: (e: KeyboardEvent) => boolean;
  run?: (e: KeyboardEvent) => void;
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-border bg-foreground/5 px-1.5 py-0.5 font-mono text-[11px] leading-none text-muted-foreground">
      {children}
    </kbd>
  );
}

const ctrl = (e: KeyboardEvent) => e.ctrlKey || e.metaKey;

export const SHORTCUTS: ShortcutItem[] = [
  // ── 绘制标注 ──
  { id: 'draw-drag', group: 'draw', desc: '按当前标签绘制标注框', keys: <span>拖拽</span> },
  {
    id: 'tag-num',
    group: 'draw',
    desc: '按数字快速切换标签（选中即进入绘制）',
    keys: (
      <span>
        <Kbd>1</Kbd>…<Kbd>9</Kbd>
      </span>
    ),
    match: (e) => /^[1-9]$/.test(e.key),
    run: (e) => {
      const t = useStore.getState().tags.tags[Number(e.key) - 1];
      if (t) useStore.getState().setActiveTag(t.name);
    },
  },
  {
    id: 'esc',
    group: 'draw',
    desc: '取消绘制或退出绘制模式',
    keys: (
      <span>
        <Kbd>Esc</Kbd> / 点击空白
      </span>
    ),
    match: (e) => e.key === 'Escape',
    run: () => {
      const s = useStore.getState();
      if (s.drag) s.setDrag(null); // 先取消进行中的拖拽/绘制
      s.exitDraw(); // 退出绘制模式 + 取消标签选中
      s.select(null);
    },
  },
  { id: 'box-context', group: 'draw', desc: '修改标签 / 删除', keys: <span>右键标注框</span> },

  // ── 编辑标注 ──
  { id: 'box-move', group: 'edit', desc: '移动标注框', keys: <span>拖动框体</span> },
  { id: 'box-resize', group: 'edit', desc: '调整大小', keys: <span>拖动锚点</span> },
  {
    id: 'box-delete',
    group: 'edit',
    desc: '删除选中框',
    keys: (
      <span>
        <Kbd>Del</Kbd> / <Kbd>Backspace</Kbd>
      </span>
    ),
    match: (e) => e.key === 'Delete' || e.key === 'Backspace',
    run: () => useStore.getState().deleteSelected(),
  },
  {
    id: 'undo',
    group: 'edit',
    desc: '撤销',
    keys: <Kbd>Ctrl+Z</Kbd>,
    match: (e) => ctrl(e) && e.key.toLowerCase() === 'z' && !e.shiftKey,
    run: () => useStore.getState().undo(),
  },
  {
    id: 'redo',
    group: 'edit',
    desc: '重做',
    keys: <Kbd>Ctrl+Y</Kbd>,
    match: (e) => ctrl(e) && e.key.toLowerCase() === 'y',
    run: () => useStore.getState().redo(),
  },

  // ── 导航与视图 ──
  {
    id: 'nav-prev-next',
    group: 'nav',
    desc: '上一张 / 下一张图片',
    keys: (
      <span>
        <Kbd>↑</Kbd> · <Kbd>↓</Kbd>
      </span>
    ),
    match: (e) => e.key === 'ArrowUp' || e.key === 'ArrowDown',
    run: (e) => {
      const s = useStore.getState();
      const idx = s.images.findIndex((i) => i.path === s.current);
      const next = idx + (e.key === 'ArrowDown' ? 1 : -1);
      if (next >= 0 && next < s.images.length) s.setCurrent(s.images[next].path);
    },
  },
  { id: 'zoom', group: 'nav', desc: '缩放画布（锚定光标位置）', keys: <span>滚轮</span> },
  { id: 'pan', group: 'nav', desc: '平移视图', keys: <span>中键拖动</span> },
];

export const SHORTCUT_GROUPS: { id: GroupId; title: string }[] = [
  { id: 'draw', title: '绘制标注' },
  { id: 'edit', title: '编辑标注' },
  { id: 'nav', title: '导航与视图' },
];

/** 全局按键分发：按注册表顺序找到第一个匹配的命令执行。
 *  仅在焦点位于可编辑元素（输入框等）或对话框打开时跳过——
 *  按钮聚焦不应吞掉快捷键（点击任意按钮后 activeElement 不再是 body）。 */
export function installShortcuts(): () => void {
  const onKey = (e: KeyboardEvent) => {
    const el = document.activeElement;
    if (
      el instanceof HTMLElement &&
      (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)
    ) {
      return;
    }
    if (document.querySelector('[role="dialog"]')) return; // 对话/浮层打开时不干扰
    const s = SHORTCUTS.find((sc) => sc.match?.(e));
    if (s?.run) {
      e.preventDefault();
      s.run(e);
    }
  };
  window.addEventListener('keydown', onKey);
  return () => window.removeEventListener('keydown', onKey);
}
