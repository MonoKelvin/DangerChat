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

/** 枚举选项展示名：首字母及 + 后一段首字母大写（"ctrl+enter" → "Ctrl+Enter"）；仅显示层，存储值不变 */
function capitalizeKey(o: string): string {
  return o
    .split('+')
    .map((seg) => (seg ? seg.charAt(0).toUpperCase() + seg.slice(1) : seg))
    .join('+');
}

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
        // 范围与步长取自 schema（后端 ConfigType 透出），前端不硬编码：
        // 步长同时决定显示小数位（float → 0.05 → 两位小数）。
        const step = field.step ?? (field.ty === 'float' ? 0.05 : 1);
        return (
          <NumberInput
            value={Number.isFinite(num) ? num : 0}
            onChange={(v) => onChange(v)}
            min={field.min}
            max={field.max}
            step={step}
          />
        );
      }
      case 'enum':
        return (
          <Combobox
            value={String(value ?? '')}
            options={field.options.map((o) => ({ value: o, label: capitalizeKey(o) }))}
            onChange={(v) => onChange(v)}
            className="w-40"
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
