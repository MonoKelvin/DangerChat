import { useEffect, useRef, useState } from 'react';
import { cn } from '../lib/utils';
import type { AlertPayload } from '../lib/types';
import { alertAction } from '../lib/commands';
import { onAlert } from '../lib/events';

/**
 * dc-alert 弹窗（FR-BRG-02/04）：常驻隐藏窗口的根组件。
 * 键盘 1/2/3/0 由全局钩子在 Cooldown 期截获代转（弹窗无焦点收不到键盘）——
 * 本组件只响应鼠标点击 + `alert://action` 事件做视觉反馈。
 */
export function AlertRoot() {
  const [payload, setPayload] = useState<AlertPayload | null>(null);
  const [phase, setPhase] = useState<'idle' | 'show' | 'allow-hint'>('idle');
  const [countdown, setCountdown] = useState(0);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    const un = onAlert((p) => {
      setPayload(p);
      setPhase('show');
      setCountdown(p.countdown_secs);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // 倒计时：归零自动「仍然发送」（已拍板默认；FR-BRG-04 默认值修订）
  useEffect(() => {
    if (phase !== 'show' || countdown <= 0) return;
    timerRef.current = window.setTimeout(() => {
      setCountdown((c) => {
        if (c <= 1) {
          void doAction('allow');
          return 0;
        }
        return c - 1;
      });
    }, 1000);
    return () => {
      if (timerRef.current != null) window.clearTimeout(timerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, countdown]);

  const doAction = async (a: 'allow' | 'cancel' | 'edit' | 'snooze') => {
    await alertAction(a);
    if (a === 'allow') {
      // allow 后提示「再按一次回车」（C-08：危信不代发）
      setPhase('allow-hint');
      window.setTimeout(() => setPhase('idle'), 2500);
    } else {
      setPhase('idle');
      setPayload(null);
    }
  };

  if (phase === 'idle' || !payload) {
    // 预渲染骨架（300ms 预算：show 时只填文本）
    return <div className="h-full w-full bg-background" data-skeleton="alert" />;
  }

  const levelColor =
    payload.level === 'block'
      ? { bar: 'bg-rose-500', text: 'text-rose-500' }
      : { bar: 'bg-amber-500', text: 'text-amber-500' };

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      {/* 级别色条 */}
      <div className={cn('h-1 w-full shrink-0', levelColor.bar)} />

      {phase === 'allow-hint' ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
          <p className="text-sm font-medium">已放行本次发送</p>
          <p className="text-xs leading-relaxed text-muted-foreground">
            请<b>再按一次回车</b>完成发送
            <br />
            （危信不会代你发送）
          </p>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className={cn('text-sm font-semibold', levelColor.text)}>
              {payload.level === 'block' ? '已阻断发送' : '发送提醒'}
            </span>
            <span className="text-[11px] tabular-nums text-muted-foreground">
              {countdown}s 后自动放行
            </span>
          </div>

          {/* 被拦消息（FR-BRG-02） */}
          <div className="mb-2 max-h-16 overflow-y-auto rounded-md bg-muted/60 px-3 py-2 text-[13px] leading-relaxed">
            {payload.draft_text || '（空）'}
          </div>

          {/* 命中原因 */}
          <ul className="mb-1 space-y-0.5 text-xs text-muted-foreground">
            {payload.reasons.slice(0, 3).map((r, i) => (
              <li key={i}>· {r}</li>
            ))}
          </ul>

          <p className="mb-3 text-[11px] text-muted-foreground">
            对象：{payload.chat_target ?? '未识别'}（按正式场景处理）
          </p>

          {/* 按钮（快捷键 1/2/3/0 由钩子代转） */}
          <div className="mt-auto grid grid-cols-3 gap-2">
            <button
              className="h-9 rounded-md bg-primary text-xs font-medium text-primary-foreground"
              onClick={() => void doAction('allow')}
            >
              仍然发送 <span className="opacity-60">1</span>
            </button>
            <button
              className="h-9 rounded-md border border-border/60 text-xs hover:bg-accent"
              onClick={() => void doAction('cancel')}
            >
              取消发送 <span className="opacity-60">2</span>
            </button>
            <button
              className="h-9 rounded-md border border-border/60 text-xs hover:bg-accent"
              onClick={() => void doAction('edit')}
            >
              返回编辑 <span className="opacity-60">3</span>
            </button>
          </div>
          <button
            className="mt-1.5 h-7 rounded-md text-[11px] text-muted-foreground hover:bg-accent"
            onClick={() => void doAction('snooze')}
          >
            本次不再提示（按 0）
          </button>
        </div>
      )}
    </div>
  );
}
