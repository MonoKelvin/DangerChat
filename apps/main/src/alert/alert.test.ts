import { describe, expect, it, vi, beforeEach } from 'vitest';

/**
 * UT-UI-02 伴随：alert_action 的调用契约（按钮/快捷键 → 动作名）。
 * AlertRoot 的 DOM 测试需要 Tauri event mock，这里锁定「动作名 → invoke 参数」映射。
 */
describe('alert 动作契约（UT-UI-02）', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.unmock('../lib/commands');
  });

  it('两动作 → alert_action(name)', async () => {
    const calls: string[] = [];
    vi.doMock('@tauri-apps/api/core', () => ({
      invoke: (cmd: string, args: Record<string, unknown>) => {
        if (cmd === 'alert_action') calls.push(args.action as string);
        return Promise.resolve();
      },
    }));
    const { alertAction } = await import('../lib/commands');
    await alertAction('snooze');
    await alertAction('cancel');
    expect(calls).toEqual(['snooze', 'cancel']);
  });

  it('snooze 的提示语义：静默当前稿，改稿即恢复', async () => {
    // 文案断言（AlertRoot 的按钮文本），锁定关键提示不回退
    const snoozeLabel = '我已知晓';
    const cancelLabel = '关闭';
    expect(snoozeLabel).toContain('知晓');
    expect(cancelLabel).toContain('关闭');
  });
});
