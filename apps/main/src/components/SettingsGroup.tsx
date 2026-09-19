import { type ReactNode } from 'react';
import { RotateCcw, type LucideIcon } from 'lucide-react';
import { cn } from '../lib/utils';

/**
 * 设置分组三件套（参考 Wanwu settings / Win11 设置）：
 * Section（页面容器）→ Group（带标题的分组卡片）→ Row（标题/副题 + 控件）。
 * Group 无阴影：明显一档的圆角背景（--group-bg），行间 hairline 分隔。
 */

export function SettingsSection({ children }: { children: ReactNode }) {
  return <div className="space-y-4">{children}</div>;
}

export function SettingsGroup({
  label,
  icon: Icon,
  children,
}: {
  label?: string;
  /** 分组标题图标（lucide 组件）：与标题同行显示，主色描边。 */
  icon?: LucideIcon;
  children: ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-2xl bg-[var(--group-bg)]">
      {label && (
        <h3 className="flex items-center gap-2 px-5 pt-4 pb-1.5 text-item font-semibold text-[var(--text-primary)]">
          {Icon && <Icon className="size-4 shrink-0 text-[var(--brand)]" strokeWidth={2.2} />}
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
  dirty,
  onReset,
  children,
}: {
  /** 省略时为纯内容行（长段落一类） */
  label?: string;
  subtitle?: string;
  /** 长内容时上下布局，默认左右 */
  stacked?: boolean;
  /** 值偏离默认时为 true：标题右上角显示 * 标记、标题后显示「还原为默认值」图标。
   *  仅对可还原的单值设置项传入；新增对象/场景/词库等集合型不传。 */
  dirty?: boolean;
  /** 点击还原图标 → 走该项正常的写入流程回到默认值。dirty 为 true 时才显示图标。 */
  onReset?: () => void;
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
          <div className="flex items-center gap-1.5">
            <p className={cn('text-sm font-medium text-[var(--text-primary)]', !subtitle && 'leading-6')}>
              {label}
              {dirty && (
                <span
                  className="ml-0.5 align-top text-[var(--brand)]"
                  title="已修改（偏离默认值）"
                  aria-label="已修改"
                >
                  *
                </span>
              )}
            </p>
            {dirty && onReset && (
              <button
                type="button"
                onClick={onReset}
                data-tip="还原为默认值"
                aria-label="还原为默认值"
                className="inline-flex size-5 items-center justify-center rounded-md text-[var(--text-tertiary)] transition-colors hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]"
              >
                <RotateCcw className="size-3.5" strokeWidth={2} />
              </button>
            )}
          </div>
          {subtitle && (
            <p className="mt-0.5 text-label leading-relaxed text-[var(--text-tertiary)]">{subtitle}</p>
          )}
        </div>
      )}
      <div className={cn('shrink-0', stacked && 'w-full', label === undefined && 'w-full')}>{children}</div>
    </div>
  );
}
