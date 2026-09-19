import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import type { AlertPayload } from '../lib/types';
import { alertAction, getConfig } from '../lib/commands';
import { onAlert } from '../lib/events';
import { AlertCard } from './AlertCard';

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
  /** 「最近对话」上下文默认折叠，避免对话框过高；用户点击展开。 */
  const [contextOpen, setContextOpen] = useState(false);
  const timerRef = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void getConfig('alert.shake').then((v) => setShakeOn(v !== false));
    void detectWinCorner().then(setCorner);
    const un = onAlert((p) => {
      setPayload(p);
      setVisible(true);
      setCountdown(p.countdown_secs);
      setContextOpen(false); // 每次新拦截都默认折叠上下文
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
  //
  // 用 ResizeObserver 观察 Body 内容高度：上下文展开/收缩、文字换行等任何内容变化都会
  // 触发它重算——只靠 [contextOpen] 依赖不够（scrollHeight 在被滚动容器裁剪后可能量不准，
  // 且展开动画/异步布局的时机也不稳）。
  useLayoutEffect(() => {
    if (!visible || !payload) return;
    const root = rootRef.current;
    const body = bodyRef.current;
    if (!root || !body) return;

    const content = contentRef.current;
    if (!content) return;

    const resize = () => {
      const chrome = root.clientHeight - body.clientHeight; // Header+Footer 固定高
      const want = chrome + content.scrollHeight; // 内容自然高（未被滚动容器裁剪）
      const h = Math.min(WIN_H_MAX, Math.max(WIN_H_MIN, Math.ceil(want)));
      void getCurrentWindow().setSize(new LogicalSize(WIN_W, h));
    };

    resize();
    // 观察内容层（无 overflow，高度即自然高）：展开/收缩、换行等任何变化都重算窗口高度
    const ro = new ResizeObserver(resize);
    ro.observe(content);
    return () => ro.disconnect();
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

  const accent = payload.level === 'block' ? 'var(--danger)' : 'var(--warning)';

  return (
    // 窗口外壳：闪烁描边（inset box-shadow）沿元素圆角走，故圆角需与系统窗口一致
    // （Win11 圆角 → 小圆角避免被裁；Win10 直角）。视觉主体交给 AlertCard（与设置页预览同源）。
    <div
      ref={rootRef}
      className={[
        'h-full bg-[var(--window-bg)] text-[var(--text-primary)]',
        corner === 'rounded' ? 'rounded-lg' : 'rounded-none',
        shakeOn ? 'alert-shake alert-flash' : '',
      ].join(' ')}
      style={{ ['--alert-accent' as string]: accent }}
    >
      <AlertCard
        payload={payload}
        countdown={countdown}
        contextOpen={contextOpen}
        onToggleContext={() => setContextOpen((v) => !v)}
        onSnooze={() => void doAction('snooze')}
        onCancel={() => void doAction('cancel')}
        bodyRef={bodyRef}
        contentRef={contentRef}
      />
    </div>
  );
}
