import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { ShieldAlert } from 'lucide-react';
import type { AlertPayload } from '../lib/types';
import { alertAction, getConfig } from '../lib/commands';
import { onAlert } from '../lib/events';

/** 窗口宽度固定，高度随内容（含滚动上限）——避免短内容留白、长内容过早滚动。 */
const WIN_W = 360;
const WIN_H_MIN = 150;
const WIN_H_MAX = 460;

/** 闪烁描边圆角：Win11 系统窗口本身有圆角，描边跟一个小圆角避免被裁；
 *  Win10 系统窗口是直角，描边也用直角贴合。默认按 Win11（更常见）。 */
type WinCorner = 'rounded' | 'square';

/** WebView2(Chromium) 高熵 UA：platformVersion 主版本 ≥ 13 = Win11，否则 Win10。 */
async function detectWinCorner(): Promise<WinCorner> {
  const uaData = (navigator as unknown as { userAgentData?: NavigatorUAData }).userAgentData;
  if (!uaData?.getHighEntropyValues) return 'rounded';
  try {
    const { platformVersion } = await uaData.getHighEntropyValues(['platformVersion']);
    const major = Number((platformVersion ?? '0').split('.')[0]);
    return major >= 13 ? 'rounded' : 'square';
  } catch {
    return 'rounded';
  }
}

interface NavigatorUAData {
  getHighEntropyValues(hints: string[]): Promise<{ platformVersion?: string }>;
}

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
  const [corner, setCorner] = useState<WinCorner>('rounded');
  const timerRef = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void getConfig('alert.shake').then((v) => setShakeOn(v !== false));
    void detectWinCorner().then(setCorner);
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

  // 内容驱动窗口高度：root 是 h-full（等于窗口高），量不出自然高度；
  // 改量 Body 的 scrollHeight（内容自然高）+ Header/Footer 固定高，钳到 [MIN, MAX]。
  // 短内容不留白、长内容才在 Body 内滚动。后端 show 给初始尺寸，这里精修。
  useLayoutEffect(() => {
    if (!visible || !payload || !rootRef.current || !bodyRef.current) return;
    const chrome = rootRef.current.clientHeight - bodyRef.current.clientHeight; // Header+Footer
    const want = chrome + bodyRef.current.scrollHeight;
    const h = Math.min(WIN_H_MAX, Math.max(WIN_H_MIN, Math.ceil(want)));
    void getCurrentWindow().setSize(new LogicalSize(WIN_W, h));
  }, [visible, payload]);

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
    // 三段式：Header/Footer 固定，仅 Body 滚动。闪烁描边（inset box-shadow）沿元素圆角走，
    // 故圆角需与系统窗口一致：Win11 系统窗口圆角 → 描边用小圆角避免被裁；Win10 直角 → 描边直角。
    <div
      ref={rootRef}
      className={[
        'flex h-full flex-col overflow-hidden bg-[var(--window-bg)] text-[var(--text-primary)]',
        corner === 'rounded' ? 'rounded-lg' : 'rounded-none',
        shakeOn ? 'alert-shake alert-flash' : '',
      ].join(' ')}
      style={{ ['--alert-accent' as string]: accent }}
    >
      {/* Header：图标 + 标题 + 倒计时，固定 */}
      <div className="flex shrink-0 items-center justify-between px-4 pb-2.5 pt-3.5">
        <div className="flex items-center gap-2">
          <span
            className="flex size-7 items-center justify-center rounded-lg"
            style={{ backgroundColor: `color-mix(in oklch, ${accent} 14%, transparent)` }}
          >
            <ShieldAlert className="size-4" style={{ color: accent }} strokeWidth={2} />
          </span>
          <span className="text-label font-semibold" style={{ color: accent }}>
            {isBlock ? '已阻断发送' : '发送提醒'}
          </span>
        </div>
        {/* 倒计时为 0 = 不自动关闭，不显示秒数标签 */}
        {countdown > 0 && (
          <span className="text-caption tabular-nums text-[var(--text-tertiary)]">{countdown}s</span>
        )}
      </div>

      {/* Body：唯一纵向滚动区，撑满剩余空间 */}
      <div ref={bodyRef} className="min-h-0 flex-1 space-y-2 overflow-y-auto px-4">
        {/* 命中原因（最影响判断，置顶） */}
        {payload.reasons.length > 0 && (
          <p className="text-caption leading-snug text-[var(--text-secondary)]">
            {payload.reasons[0]}
          </p>
        )}

        {/* 被拦消息 */}
        <div className="rounded-lg bg-[var(--card-bg)] px-3 py-2.5 text-label leading-relaxed shadow-[var(--shadow-sm)]">
          <div className="whitespace-pre-wrap break-words">{payload.draft_text || '（空）'}</div>
        </div>

        {/* 最近聊天上下文（辅助判断） */}
        {payload.chat_context && (
          <div className="rounded-lg bg-[var(--card-bg)] px-3 py-2 text-caption leading-relaxed text-[var(--text-secondary)]">
            <div className="mb-1 font-medium text-[var(--text-tertiary)]">最近对话</div>
            <div className="whitespace-pre-wrap break-words">{payload.chat_context}</div>
          </div>
        )}
      </div>

      {/* Footer：两个动作，固定底部 */}
      <div className="grid shrink-0 grid-cols-2 gap-2 px-4 pb-3.5 pt-2.5">
        <button
          className="h-9 rounded-lg bg-[var(--primary)] text-xs font-medium text-[var(--primary-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--primary-hover)]"
          onClick={() => void doAction('snooze')}
          data-tip="本次对话不再弹出"
          data-tip-side="top"
          data-tip-delay="50"
        >
          我已知晓
        </button>
        <button
          className="h-9 rounded-lg bg-[var(--card-bg)] text-xs text-[var(--text-secondary)] shadow-[var(--shadow-sm)] transition-colors hover:text-[var(--text-primary)]"
          onClick={() => void doAction('cancel')}
          data-tip="下次仍会弹出"
          data-tip-side="top"
          data-tip-delay="50"
        >
          关闭
        </button>
      </div>
    </div>
  );
}
