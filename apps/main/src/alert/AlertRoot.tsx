import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ShieldAlert } from 'lucide-react';
import type { AlertPayload } from '../lib/types';
import { alertAction, getConfig } from '../lib/commands';
import { onAlert } from '../lib/events';

/**
 * dc-alert 拦截弹窗：常驻隐藏窗口的根组件。
 * 两个动作：「我已知晓」= 静默当前草稿（改稿即恢复守护）；「关闭」= 解除本轮
 * 拦截（草稿未变时下次发送会再次弹出）。超时等同「关闭」。
 * 任何路径（按钮/超时/异常）都必须隐藏窗口——曾经只改 React 状态导致窗口残留。
 */
export function AlertRoot() {
  const [payload, setPayload] = useState<AlertPayload | null>(null);
  const [visible, setVisible] = useState(false);
  const [countdown, setCountdown] = useState(0);
  const [shakeOn, setShakeOn] = useState(true);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    void getConfig('alert.shake').then((v) => setShakeOn(v !== false));
    const un = onAlert((p) => {
      setPayload(p);
      setVisible(true);
      setCountdown(p.countdown_secs);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // 倒计时归零 →「关闭」
  useEffect(() => {
    if (!visible || countdown <= 0) return;
    timerRef.current = window.setTimeout(() => {
      setCountdown((c) => {
        if (c <= 1) {
          void doAction('cancel');
          return 0;
        }
        return c - 1;
      });
    }, 1000);
    return () => {
      if (timerRef.current != null) window.clearTimeout(timerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, countdown]);

  const doAction = async (a: 'snooze' | 'cancel') => {
    if (timerRef.current != null) window.clearTimeout(timerRef.current);
    try {
      await alertAction(a);
    } catch {
      // 后端失败也必须关窗（窗口残留比动作失败更干扰用户）
    }
    setVisible(false);
    setPayload(null);
    void getCurrentWindow().hide();
  };

  if (!visible || !payload) {
    return <div className="h-full w-full bg-[var(--window-bg)]" data-skeleton="alert" />;
  }

  const isBlock = payload.level === 'block';
  const accent = isBlock ? 'var(--danger)' : 'var(--warning)';

  return (
    <div
      className={[
        'flex h-full flex-col bg-[var(--window-bg)] p-4 text-[var(--text-primary)]',
        shakeOn ? 'alert-shake alert-flash' : '',
      ].join(' ')}
      style={{ ['--alert-accent' as string]: accent }}
    >
      {/* 头部 */}
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span
            className="flex size-7 items-center justify-center rounded-lg"
            style={{ backgroundColor: `color-mix(in oklch, ${accent} 14%, transparent)` }}
          >
            <ShieldAlert className="size-4" style={{ color: accent }} strokeWidth={2} />
          </span>
          <span className="text-[13px] font-semibold" style={{ color: accent }}>
            {isBlock ? '已阻断发送' : '发送提醒'}
          </span>
        </div>
        <span className="text-[11px] tabular-nums text-[var(--text-tertiary)]">{countdown}s</span>
      </div>

      {/* 被拦消息 */}
      <div className="mb-2.5 max-h-24 flex-1 overflow-y-auto rounded-lg bg-[var(--card-bg)] px-3 py-2.5 text-[13px] leading-relaxed shadow-[var(--shadow-sm)]">
        {payload.draft_text || '（空）'}
      </div>

      {/* 命中原因 */}
      {payload.reasons.length > 0 && (
        <p className="mb-3 truncate text-[11px] text-[var(--text-secondary)]">
          {payload.reasons[0]}
        </p>
      )}

      {/* 两个动作 */}
      <div className="grid grid-cols-2 gap-2">
        <button
          className="h-9 rounded-lg bg-[var(--primary)] text-xs font-medium text-[var(--primary-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--primary-hover)]"
          onClick={() => void doAction('snooze')}
        >
          我已知晓
        </button>
        <button
          className="h-9 rounded-lg bg-[var(--card-bg)] text-xs text-[var(--text-secondary)] shadow-[var(--shadow-sm)] transition-colors hover:text-[var(--text-primary)]"
          onClick={() => void doAction('cancel')}
        >
          关闭
        </button>
      </div>
    </div>
  );
}
