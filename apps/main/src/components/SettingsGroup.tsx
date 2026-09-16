import { type ReactNode } from 'react';
import { cn } from '../lib/utils';

/**
 * 设置分组三件套（参考 Wanwu settings / Win11 设置）：
 * Section（页面容器）→ Group（带标题的分组卡片）→ Row（标题/副题 + 控件）。
 * Group 无阴影：明显一档的圆角背景（--group-bg），行间 hairline 分隔。
 */

export function SettingsSection({ children }: { children: ReactNode }) {
  return <div className="space-y-4">{children}</div>;
}

export function SettingsGroup({ label, children }: { label?: string; children: ReactNode }) {
  return (
    <section className="overflow-hidden rounded-2xl bg-[var(--group-bg)]">
      {label && (
        <h3 className="px-5 pt-4 pb-1 text-[13px] font-semibold text-[var(--text-secondary)]">
          {label}
        </h3>
      )}
      <div className={label ? 'pb-1.5' : 'py-1.5'}>{children}</div>
    </section>
  );
}

export function SettingsRow({
  label,
  subtitle,
  stacked,
  children,
}: {
  /** 省略时为纯内容行（长段落一类） */
  label?: string;
  subtitle?: string;
  /** 长内容时上下布局，默认左右 */
  stacked?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        'border-b border-[var(--divider)] px-5 py-3.5 last:border-b-0',
        stacked ? 'flex flex-col items-stretch gap-4' : 'flex items-center justify-between gap-8',
      )}
    >
      {label !== undefined && (
        <div className="min-w-0">
          <p className={cn('text-sm font-medium text-[var(--text-primary)]', !subtitle && 'leading-6')}>
            {label}
          </p>
          {subtitle && (
            <p className="mt-0.5 text-[13px] leading-relaxed text-[var(--text-tertiary)]">{subtitle}</p>
          )}
        </div>
      )}
      <div className={cn('shrink-0', stacked && 'w-full', label === undefined && 'w-full')}>{children}</div>
    </div>
  );
}
