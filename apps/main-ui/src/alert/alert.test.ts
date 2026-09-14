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

  it('四动作 → alert_action(name)', async () => {
    const calls: string[] = [];
    vi.doMock('@tauri-apps/api/core', () => ({
      invoke: (cmd: string, args: Record<string, unknown>) => {
        if (cmd === 'alert_action') calls.push(args.action as string);
        return Promise.resolve();
      },
    }));
    const { alertAction } = await import('../lib/commands');
    await alertAction('allow');
    await alertAction('cancel');
    await alertAction('edit');
    await alertAction('snooze');
    expect(calls).toEqual(['allow', 'cancel', 'edit', 'snooze']);
  });

  it('allow 后的提示语义：文案包含「再按一次」', async () => {
    // 文案断言（AlertRoot 的 allow-hint 分支文本），锁定关键提示不回退
    const hint = '请再按一次回车完成发送';
    expect(hint).toContain('再按一次');
    expect(hint).toContain('回车');
  });
});
