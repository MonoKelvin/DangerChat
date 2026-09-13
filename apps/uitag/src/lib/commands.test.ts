import { describe, expect, it } from 'vitest';
import { applyCmd, History } from './commands';
import type { AnnoBox } from './types';

const box = (tag: string, x = 0, y = 0, w = 10, h = 10): AnnoBox => ({ tag, x, y, w, h });
const P = 'img/a.png';

describe('applyCmd', () => {
  it('add 的 do/undo 互逆', () => {
    const a: Record<string, AnnoBox[]> = {};
    const cmd = { kind: 'add' as const, path: P, box: box('msg_input', 1, 2, 3, 4) };
    const done = applyCmd(a, cmd, 'do');
    expect(done[P]).toHaveLength(1);
    const undone = applyCmd(done, cmd, 'undo');
    expect(undone[P]).toHaveLength(0);
  });

  it('delete 的 do/undo 互逆（按原索引回插）', () => {
    const a = { [P]: [box('a'), box('b'), box('c')] };
    const cmd = { kind: 'delete' as const, path: P, index: 1, box: box('b') };
    const done = applyCmd(a, cmd, 'do');
    expect(done[P].map((b) => b.tag)).toEqual(['a', 'c']);
    const undone = applyCmd(done, cmd, 'undo');
    expect(undone[P].map((b) => b.tag)).toEqual(['a', 'b', 'c']);
  });

  it('transform 与 relabel 的 do/undo 互逆', () => {
    const a = { [P]: [box('a', 0, 0, 10, 10)] };
    const t = {
      kind: 'transform' as const,
      path: P,
      index: 0,
      before: box('a', 0, 0, 10, 10),
      after: box('a', 5, 5, 20, 20),
    };
    const moved = applyCmd(a, t, 'do');
    expect(moved[P][0]).toEqual(box('a', 5, 5, 20, 20));
    expect(applyCmd(moved, t, 'undo')[P][0]).toEqual(box('a', 0, 0, 10, 10));

    const r = { kind: 'relabel' as const, path: P, index: 0, before: 'a', after: 'b' };
    const relabeled = applyCmd(moved, r, 'do');
    expect(relabeled[P][0].tag).toBe('b');
    expect(applyCmd(relabeled, r, 'undo')[P][0].tag).toBe('a');
  });

  it('不修改原表（不可变性）', () => {
    const a = { [P]: [box('a')] };
    const cmd = { kind: 'add' as const, path: P, box: box('b') };
    applyCmd(a, cmd, 'do');
    expect(a[P]).toHaveLength(1);
  });
});

/// UT-TAG-04：命令模式栈——撤销/重做序列与 push 清空 redo 语义。
describe('History（UT-TAG-04）', () => {
  it('undo/redo 序列往返', () => {
    const h = new History();
    let annos: Record<string, AnnoBox[]> = {};
    const c1 = { kind: 'add' as const, path: P, box: box('a') };
    const c2 = { kind: 'add' as const, path: P, box: box('b') };

    annos = applyCmd(annos, c1, 'do'); h.push(c1);
    annos = applyCmd(annos, c2, 'do'); h.push(c2);
    expect(annos[P]).toHaveLength(2);
    expect(h.canUndo).toBe(true);

    const u1 = h.popUndo(); annos = applyCmd(annos, u1!, 'undo');
    expect(annos[P]).toHaveLength(1);
    const u2 = h.popUndo(); annos = applyCmd(annos, u2!, 'undo');
    expect(annos[P]).toHaveLength(0);
    expect(h.canUndo).toBe(false);
    expect(h.canRedo).toBe(true);

    const r1 = h.popRedo(); annos = applyCmd(annos, r1!, 'do');
    expect(annos[P]).toHaveLength(1);
    const r2 = h.popRedo(); annos = applyCmd(annos, r2!, 'do');
    expect(annos[P]).toHaveLength(2);
    expect(h.canRedo).toBe(false);
  });

  it('新命令清空 redo 栈', () => {
    const h = new History();
    const c1 = { kind: 'add' as const, path: P, box: box('a') };
    const c2 = { kind: 'add' as const, path: P, box: box('b') };
    const c3 = { kind: 'add' as const, path: P, box: box('c') };
    h.push(c1); h.push(c2);
    h.popUndo();
    expect(h.canRedo).toBe(true);
    h.push(c3);
    expect(h.canRedo).toBe(false);
  });

  it('栈上限 100：最老的被挤掉', () => {
    const h = new History();
    for (let i = 0; i < 130; i++) {
      h.push({ kind: 'add', path: P, box: box(`t${i}`) });
    }
    let n = 0;
    while (h.popUndo()) n++;
    expect(n).toBe(100);
  });
});
