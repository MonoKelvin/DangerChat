import type { ConfigFieldDto } from '../lib/types';
import { Switch } from '../components/Switch';
import { Combobox } from '../components/Combobox';
import { NumberInput } from '../components/NumberInput';
import { SettingsRow } from '../components/SettingsGroup';

/** SchemaField：ConfigType → 控件（schema 驱动渲染，新增配置项零前端改动，FR-UI-05）。 */

interface FieldProps {
  field: ConfigFieldDto;
  value: unknown;
  onChange: (v: unknown) => void;
}

const input =
  'h-9 w-full rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 placeholder:text-[var(--text-tertiary)] hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]';

export function SchemaField({ field, value, onChange }: FieldProps) {
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
        const step = field.ty === 'float' ? 0.05 : 1;
        return (
          <NumberInput
            value={Number.isFinite(num) ? num : 0}
            onChange={(v) => onChange(v)}
            step={step}
          />
        );
      }
      case 'enum':
        return (
          <Combobox
            value={String(value ?? '')}
            options={field.options.map((o) => ({ value: o, label: o }))}
            onChange={(v) => onChange(v)}
            className="w-52"
          />
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
    <SettingsRow label={field.label} subtitle={field.help}>
      {control}
    </SettingsRow>
  );
}
