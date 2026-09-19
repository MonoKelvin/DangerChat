import { type ReactNode } from 'react';
import { ShieldAlert, ChevronDown } from 'lucide-react';
import type { AlertPayload } from '../lib/types';

/**
 * 拦截弹窗视觉主体（单一来源）：Header（图标+标题+倒计时）/ Body（原因+被拦消息+可折叠上下文）/ Footer（两动作）。
 *
 * 抽出的原因：真实弹窗（AlertRoot，独立 alert 窗口）与设置页预览要长得一模一样，
 * 视觉只维护这一处。外壳差异（窗口尺寸测量/隐藏/闪烁描边、预览的静态外框）由各调用方包裹。
 */
export interface AlertCardProps {
  payload: AlertPayload;
  /** 倒计时秒数；<=0 不显示 */
  countdown?: number;
  /** 「最近对话」是否展开 */
  contextOpen: boolean;
  onToggleContext: () => void;
  /** 「我已知晓」= 静默本轮草稿 */
  onSnooze?: () => void;
  /** 「关闭」= 解除本轮拦截 */
  onCancel?: () => void;
  /** Body 滚动容器 ref（AlertRoot 用它算 Header/Footer 固定高） */
  bodyRef?: React.Ref<HTMLDivElement>;
  /** Body 内容层 ref（无 overflow，自然高度——AlertRoot 用 ResizeObserver 观察它定窗口高度） */
  contentRef?: React.Ref<HTMLDivElement>;
}

export function AlertCard({
  payload,
  countdown = 0,
  contextOpen,
  onToggleContext,
  onSnooze,
  onCancel,
  bodyRef,
  contentRef,
}: AlertCardProps) {
  const isBlock = payload.level === 'block';
  const accent = isBlock ? 'var(--danger)' : 'var(--warning)';

  return (
    <div className="flex h-full flex-col overflow-hidden" style={{ ['--alert-accent' as string]: accent }}>
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

      {/* Body：唯一纵向滚动区，撑满剩余空间。内容包一层 contentRef（无 overflow，
          自然高度）供窗口高度测量——直接量滚动容器会被裁剪，量不到内容增长。 */}
      <div ref={bodyRef} className="min-h-0 flex-1 overflow-y-auto px-4">
        <div ref={contentRef} className="space-y-2">
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

          {/* 最近聊天上下文（辅助判断）：默认折叠，点标题行展开/收缩 */}
          {payload.chat_context && (
            <div className="rounded-lg bg-[var(--card-bg)] px-3 py-2 text-caption leading-relaxed text-[var(--text-secondary)]">
              <button
                type="button"
                onClick={onToggleContext}
                className="flex w-full items-center justify-between font-medium text-[var(--text-tertiary)] transition-colors hover:text-[var(--text-secondary)]"
                aria-expanded={contextOpen}
              >
                <span>最近对话</span>
                <ChevronDown
                  className={`size-4 shrink-0 transition-transform duration-200 ${contextOpen ? 'rotate-180' : ''}`}
                  strokeWidth={2}
                />
              </button>
              {contextOpen && (
                <div className="mt-1 whitespace-pre-wrap break-words">{payload.chat_context}</div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Footer：两个动作，固定底部 */}
      <div className="grid shrink-0 grid-cols-2 gap-2 px-4 pb-3.5 pt-2.5">
        <button
          className="h-9 rounded-lg bg-[var(--primary)] text-xs font-medium text-[var(--primary-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--primary-hover)]"
          onClick={onSnooze}
          data-tip="本次对话不再弹出"
          data-tip-side="top"
          data-tip-delay="50"
        >
          我已知晓
        </button>
        <button
          className="h-9 rounded-lg bg-[var(--card-bg)] text-xs text-[var(--text-secondary)] shadow-[var(--shadow-sm)] transition-colors hover:text-[var(--text-primary)]"
          onClick={onCancel}
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

/** 预览用示例载荷（设置页「拦截效果预览」）：一段会触发拦截的示例文本 + 最近对话上下文。 */
export const PREVIEW_ALERT: AlertPayload = {
  level: 'block',
  score: 0.86,
  reasons: ['与「正式」场景语义不匹配，可能造成误会或冒犯'],
  chat_target: '张经理',
  chat_context: '张经理：这个方案下午三点前给我\n你：好的，马上整理',
  draft_text: '这点破事都要催，烦不烦啊',
  draft_fingerprint: 0,
  draft_epoch: 0,
  countdown_secs: 0,
};

/** 预览外框：模拟独立弹窗的窗口边框/圆角/阴影，让预览与真实弹窗观感一致。 */
export function AlertPreviewFrame({ children }: { children: ReactNode }) {
  return (
    <div className="mx-auto w-[360px] overflow-hidden rounded-lg border border-[var(--glass-border)] bg-[var(--window-bg)] text-[var(--text-primary)] shadow-[var(--shadow-lg)]">
      {children}
    </div>
  );
}
