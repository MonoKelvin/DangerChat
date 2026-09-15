import type { ConfigFieldDto } from '../lib/types';
import { cn } from '../lib/utils';
import { Switch } from '../components/Switch';

/** SchemaField：ConfigType → 控件（schema 驱动渲染，新增配置项零前端改动，FR-UI-05）。 */

interface FieldProps {
  field: ConfigFieldDto;
  value: unknown;
  onChange: (v: unknown) => void;
}

const input =
  'h-10 w-full rounded-lg border border-[var(--border)] bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 placeholder:text-[var(--text-tertiary)] hover:border-[var(--border-strong)] focus:border-[var(--primary)] focus:ring-2 focus:ring-[var(--focus-ring)]';

export function SchemaField({ field, value, onChange }: FieldProps) {
  const label = (
    <div className="min-w-0 flex-1">
      <p className="text-[13px] font-medium leading-snug text-[var(--text-primary)]">{field.label}</p>
      {field.help && (
        <p className="mt-1 text-xs leading-relaxed text-[var(--text-secondary)]">{field.help}</p>
      )}
    </div>
  );

  const control = (() => {
    switch (field.ty) {
      case 'bool':
        return (
          <Switch
            checked={value === true}
            onChange={(checked) => onChange(checked)}
            label={field.label}
          />
        );
      case 'int':
      case 'float': {
        const num = typeof value === 'number' ? value : Number(value ?? 0);
        return (
          <input
            type="number"
            className={cn(input, 'w-36 text-right tabular-nums')}
            value={Number.isFinite(num) ? num : 0}
            step={field.ty === 'float' ? 0.05 : 1}
            onChange={(e) => {
              const v = field.ty === 'float' ? parseFloat(e.target.value) : parseInt(e.target.value, 10);
              onChange(Number.isFinite(v) ? v : 0);
            }}
          />
        );
      }
      case 'enum':
        return (
          <select
            className={cn(input, 'w-52 cursor-pointer')}
            value={String(value ?? '')}
            onChange={(e) => onChange(e.target.value)}
          >
            {(field.default as unknown as { options?: string[] })?.options?.map?.((o) => (
              <option key={o} value={o}>
                {o}
              </option>
            )) ?? <option value={String(value ?? '')}>{String(value ?? '')}</option>}
          </select>
        );
      case 'strlist': {
        const list = Array.isArray(value) ? (value as string[]) : [];
        return (
          <input
            className={input}
            value={list.join('，')}
            placeholder="多项用中文逗号分隔"
            onChange={(e) => onChange(e.target.value.split('，').map((s) => s.trim()).filter(Boolean))}
          />
        );
      }
      case 'path':
      case 'text':
      default:
        return (
          <input
            className={input}
            value={String(value ?? '')}
            placeholder={field.ty === 'path' ? '文件路径' : ''}
            onChange={(e) => onChange(e.target.value)}
          />
        );
    }
  })();

  return (
    <div className="group flex items-center justify-between gap-8 rounded-lg bg-[var(--card-bg)] px-5 py-4 shadow-[var(--shadow-sm)] transition-all duration-150 hover:shadow-[var(--shadow-md)]">
      {label}
      <div className="shrink-0">{control}</div>
    </div>
  );
}
