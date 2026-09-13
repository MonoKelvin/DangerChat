import type { AnnoBox } from './types';

/**
 * 撤销/重做的最小数据单元：命令即数据。
 * 每个命令记录 before/after 快照（而非增量 delta），apply 时按方向切换状态，
 * 移动/缩放/改标签天然可逆且实现简单。
 */
export type Cmd =
  | { kind: 'add'; path: string; box: AnnoBox }
  | { kind: 'delete'; path: string; index: number; box: AnnoBox }
  | { kind: 'transform'; path: string; index: number; before: AnnoBox; after: AnnoBox }
  | { kind: 'relabel'; path: string; index: number; before: string; after: string };

export type Direction = 'do' | 'undo';

/** 把一条命令应用到标注表（纯函数，返回新表）。 */
export function applyCmd(
  annos: Record<string, AnnoBox[]>,
  cmd: Cmd,
  dir: Direction,
): Record<string, AnnoBox[]> {
  const list = annos[cmd.path] ?? [];
  const next = [...list];
  switch (cmd.kind) {
    case 'add':
      if (dir === 'do') next.push(cmd.box);
      else {
        const i = next.findIndex((b) => b === cmd.box);
        if (i >= 0) next.splice(i, 1);
      }
      break;
    case 'delete':
      if (dir === 'do') next.splice(cmd.index, 1);
      else next.splice(cmd.index, 0, cmd.box);
      break;
    case 'transform':
      next[cmd.index] = dir === 'do' ? cmd.after : cmd.before;
      break;
    case 'relabel':
      next[cmd.index] = { ...next[cmd.index], tag: dir === 'do' ? cmd.after : cmd.before };
      break;
  }
  return { ...annos, [cmd.path]: next };
}

export const HISTORY_LIMIT = 100;

/** 双栈历史：push 清空 redo（常规编辑器语义）。 */
export class History {
  private undoStack: Cmd[] = [];
  private redoStack: Cmd[] = [];

  push(cmd: Cmd): void {
    this.undoStack.push(cmd);
    if (this.undoStack.length > HISTORY_LIMIT) this.undoStack.shift();
    this.redoStack = [];
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  /** 取出待撤销命令（不应用——应用由调用方用 applyCmd 做）。 */
  popUndo(): Cmd | null {
    const cmd = this.undoStack.pop() ?? null;
    if (cmd) this.redoStack.push(cmd);
    return cmd;
  }

  popRedo(): Cmd | null {
    const cmd = this.redoStack.pop() ?? null;
    if (cmd) this.undoStack.push(cmd);
    return cmd;
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
  }
}
