import type { ConfigFieldDto } from '../lib/types';
import { cn } from '../lib/utils';

/** SchemaField：ConfigType → 控件（schema 驱动渲染，新增配置项零前端改动，FR-UI-05）。 */

interface FieldProps {
  field: ConfigFieldDto;
  value: unknown;
  onChange: (v: unknown) => void;
}

const input =
  'h-8 w-full rounded-md border border-border/60 bg-background px-2.5 text-sm outline-none transition-colors focus:border-ring focus:ring-1 focus:ring-ring';

export function SchemaField({ field, value, onChange }: FieldProps) {
  const label = (
    <div className="min-w-0">
      <p className="text-sm font-medium">{field.label}</p>
      {field.help && <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{field.help}</p>}
    </div>
  );

  const control = (() => {
    switch (field.ty) {
      case 'bool':
        return (
          <button
            role="switch"
            aria-checked={value === true}
            onClick={() => onChange(!(value === true))}
            className={cn(
              'relative h-6 w-11 shrink-0 rounded-full transition-colors',
              value === true ? 'bg-primary' : 'bg-muted-foreground/30',
            )}
          >
            <span
              className={cn(
                'absolute top-0.5 size-5 rounded-full bg-white shadow transition-all',
                value === true ? 'left-[22px]' : 'left-0.5',
              )}
            />
          </button>
        );
      case 'int':
      case 'float': {
        const num = typeof value === 'number' ? value : Number(value ?? 0);
        return (
          <input
            type="number"
            className={cn(input, 'w-32 text-right tabular-nums')}
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
            className={cn(input, 'w-48')}
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
            onChange={(e) => onChange(e.target.value)}
          />
        );
    }
  })();

  return (
    <div className="flex items-center justify-between gap-6 rounded-lg border border-border/40 bg-card/50 px-4 py-3">
      {label}
      {control}
    </div>
  );
}
