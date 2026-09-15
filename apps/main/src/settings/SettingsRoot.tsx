import { useEffect, useMemo, useState, useRef } from 'react';
import { Shield, ShieldAlert, ShieldOff, Plus, Trash2, FolderOpen, FolderCog, Moon, Sun, Monitor } from 'lucide-react';
import { cn } from '../lib/utils';
import type { ConfigFieldDto, StatusPayload } from '../lib/types';
import * as api from '../lib/commands';
import { onStats, onStatus } from '../lib/events';
import { SchemaField } from './SchemaField';
import { Switch } from '../components/Switch';
import { IconButton } from '../components/IconButton';
import { Combobox, type ComboOption } from '../components/Combobox';
import { SettingsGroup, SettingsRow, SettingsSection } from '../components/SettingsGroup';
import { AboutHero } from './AboutHero';
import {
  ACCENTS,
  getAccent,
  getTheme,
  setAccent,
  setTheme,
  onSystemChange,
  applyTheme,
  type Theme,
} from '../lib/theme';

/** 分类导航（FR-UI-03）：按用户视角归组——围绕微信的日常开关 + 独立词库 +
 *  面向开发者的技术参数（高级设置）。 */
const CATEGORY_ORDER = ['通用', '微信防护', '违禁词库', '高级设置', '关于'] as const;
type Category = (typeof CATEGORY_ORDER)[number];

/** 技术参数键（判定有效期/防抖/模型阈值等一般人无需关心的）→ 高级设置 */
const ADVANCED_KEYS = new Set([
  'guard.verdict_ttl_ms',
  'guard.fast_debounce_ms',
  'guard.allow_once_timeout_ms',
  'guard.foreground_debounce_ms',
  'pipeline.heartbeat_ms',
]);

/** 内置目标程序（下拉选择，不允许输入） */
const TARGET_APPS: ComboOption[] = [
  { value: 'Weixin.exe', label: '微信' },
  { value: 'notepad.exe', label: '记事本', hint: '验证用' },
];

/** 微信防护页的分组顺序与标题 */
const WECHAT_GROUPS: { title: string; keys: string[] }[] = [
  { title: '目标程序', keys: ['target.process_name'] },
  { title: '拦截行为', keys: ['guard.enabled', 'guard.send_key', 'guard.pause_hotkey', 'alert.timeout_secs'] },
  { title: '语义判定', keys: ['sem.l2_enabled', 'sem.threshold.formal', 'sem.threshold.casual'] },
];

/** 高级设置的分组顺序与标题 */
const ADVANCED_GROUPS: { title: string; keys: string[] }[] = [
  { title: '拦截引擎', keys: [...ADVANCED_KEYS] },
  { title: '区域识别', keys: ['layout.model', 'layout.conf_threshold', 'layout.nms_iou'] },
  { title: '文字识别', keys: ['ocr.upscale', 'ocr.min_conf', 'ocr.noise_words'] },
];

export function SettingsRoot() {
  const [fields, setFields] = useState<ConfigFieldDto[]>([]);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [status, setStatus] = useState<StatusPayload | null>(null);
  const [stats, setStats] = useState<{ today_blocked: number } | null>(null);
  const [active, setActive] = useState<Category>('通用');

  useEffect(() => {
    void (async () => {
      const schema = await api.getConfigSchema();
      setFields(schema);
      const vs: Record<string, unknown> = {};
      await Promise.all(
        schema.map(async (f) => {
          vs[f.key] = await api.getConfig(f.key);
        }),
      );
      setValues(vs);
      setStatus(await api.getGuardStatus());
    })();
    const u1 = onStatus(setStatus);
    const u2 = onStats(setStats);
    return () => {
      void u1.then((f) => f());
      void u2.then((f) => f());
    };
  }, []);

  const byKey = useMemo(() => new Map(fields.map((f) => [f.key, f])), [fields]);

  const change = (key: string, v: unknown) => {
    setValues((prev) => ({ ...prev, [key]: v }));
    void api.setConfig(key, v);
  };

  const StateIcon =
    status?.state === 'active' && status.target_found ? Shield :
    status?.state === 'paused' ? ShieldOff :
    ShieldAlert;

  const stateColor =
    status?.state === 'active' && status.target_found
      ? 'text-[var(--success)]'
      : status?.state === 'paused'
        ? 'text-[var(--warning)]'
        : 'text-[var(--text-tertiary)]';

  const stateText =
    status == null
      ? '初始化中'
      : status.state === 'active'
        ? status.target_found
          ? '守护中'
          : '等待目标窗口'
        : status.state === 'paused'
          ? '已暂停'
          : status.state === 'suspended'
            ? '挂起'
            : '冷却';

  /** logo 动态变色：同一张图 CSS 色相旋转/去饱和（与托盘 Rust 端 tint_for 同一映射）。
   *  守护中 = 主题色；挂起 = 浅灰；暂停(禁用) = 深灰；未发现目标 = 去饱和。 */
  const accent = getAccent();
  const hueShift = `hue-rotate(${Math.round(accent.hue - 11)}deg)`;
  const logoFilter =
    status == null
      ? 'saturate(0.12)'
      : status.state === 'active'
        ? status.target_found
          ? hueShift
          : 'saturate(0.12)'
        : status.state === 'paused'
          ? 'saturate(0.06) brightness(0.65)'
          : status.state === 'suspended'
            ? 'saturate(0.1) brightness(1.45)'
            : 'saturate(0.1) brightness(1.45)';

  /** 渲染一组 schema 字段（目标程序特殊渲染为内置下拉） */
  const renderField = (key: string) => {
    if (key === 'target.process_name') {
      return (
        <TargetAppRow
          key={key}
          value={values[key] ?? byKey.get(key)?.default}
          onChange={(v) => change(key, v)}
        />
      );
    }
    const f = byKey.get(key);
    if (!f) return null;
    return (
      <SchemaField
        key={f.key}
        field={f}
        value={values[f.key] ?? f.default}
        onChange={(v) => change(f.key, v)}
      />
    );
  };

  return (
    <div className="flex h-full">
      {/* 左侧分类导航：与标题栏同底色（Win11 设置式分层） */}
      <nav className="flex w-56 shrink-0 flex-col bg-[var(--sidebar-bg)] px-3 pb-4">
        <div className="mb-5 flex items-center gap-3 px-2 pt-1">
          <img
            src="app-icon.png"
            alt="危信"
            className="size-9 rounded-xl object-cover shadow-[var(--shadow-sm)] transition-[filter] duration-300"
            style={{ filter: logoFilter }}
          />
          <div className="flex flex-1 flex-col leading-tight">
            <span className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">危信</span>
            <div className="mt-0.5 flex items-center gap-1.5 text-xs">
              <StateIcon className={cn('size-3.5', stateColor)} strokeWidth={2} />
              <span className="text-[var(--text-tertiary)]">{stateText}</span>
            </div>
          </div>
        </div>

        <div className="space-y-0.5">
          {CATEGORY_ORDER.map((c) => (
            <button
              key={c}
              onClick={() => setActive(c)}
              className={cn(
                'w-full rounded-lg px-3 py-2 text-left text-sm font-medium transition-all duration-150',
                active === c
                  ? 'bg-[var(--panel-bg)] text-[var(--text-primary)] shadow-[var(--shadow-md)]'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
              )}
            >
              {c}
            </button>
          ))}
        </div>

        {stats && (
          <div className="mt-auto rounded-xl bg-[var(--group-bg)] px-4 py-3">
            <div className="flex items-baseline gap-1.5">
              <span className="text-xs text-[var(--text-tertiary)]">今日拦截</span>
              <strong className="text-lg font-semibold tabular-nums tracking-tight text-[var(--brand)]">
                {stats.today_blocked}
              </strong>
              <span className="text-xs text-[var(--text-tertiary)]">次</span>
            </div>
          </div>
        )}
      </nav>

      {/* 右侧设置区：提亮一档（Win11：左导航灰、右内容白） */}
      <div className="min-w-0 flex-1 overflow-y-auto rounded-tl-xl bg-[var(--panel-bg)] px-8 py-6">
        {active === '通用' && <GeneralPage />}
        {active === '微信防护' && (
          <div className="mx-auto max-w-2xl">
            <h2 className="mb-5 text-xl font-semibold tracking-tight text-[var(--text-primary)]">微信防护</h2>
            <SettingsSection>
              {WECHAT_GROUPS.map((g) => (
                <SettingsGroup key={g.title} label={g.title}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
              <ContactsGroup />
            </SettingsSection>
          </div>
        )}
        {active === '违禁词库' && <RulesEditor />}
        {active === '高级设置' && (
          <div className="mx-auto max-w-2xl">
            <h2 className="mb-1 text-xl font-semibold tracking-tight text-[var(--text-primary)]">高级设置</h2>
            <p className="mb-5 text-[13px] text-[var(--text-tertiary)]">
              面向开发与调试的技术参数，日常使用无需调整。
            </p>
            <SettingsSection>
              {ADVANCED_GROUPS.map((g) => (
                <SettingsGroup key={g.title} label={g.title}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
              <LogsGroup />
            </SettingsSection>
          </div>
        )}
        {active === '关于' && <AboutPage />}
      </div>
    </div>
  );
}

/* ── 通用：外观 + 系统 ── */

function GeneralPage() {
  return (
    <div className="mx-auto max-w-2xl">
      <h2 className="mb-5 text-xl font-semibold tracking-tight text-[var(--text-primary)]">通用</h2>
      <SettingsSection>
        <AppearanceGroup />
        <SettingsGroup label="系统">
          <AutostartRow />
          <DataDirRow />
        </SettingsGroup>
      </SettingsSection>
    </div>
  );
}

/** 外观：主题三态分段 + 主题色色板 + 自定义鼠标指针开关 */
function AppearanceGroup() {
  const [theme, setThemeState] = useState<Theme>(getTheme);
  const [accentId, setAccentId] = useState(getAccent().id);
  const [cursorFx, setCursorFx] = useState(() => localStorage.getItem('main-ui:cursorfx') !== 'off');

  useEffect(() => {
    applyTheme(theme);
    return theme === 'system' ? onSystemChange(() => applyTheme(theme)) : undefined;
  }, [theme]);

  // 鼠标指针开关 → 挂载/卸载 CursorFx（渲染由 App 层根据本设置全局控制，
  // 这里只写 localStorage + 派发自定义事件通知）
  const toggleCursorFx = (on: boolean) => {
    setCursorFx(on);
    localStorage.setItem('main-ui:cursorfx', on ? 'on' : 'off');
    window.dispatchEvent(new CustomEvent('main:cursorfx', { detail: on }));
  };

  const themeOptions: { value: Theme; label: string; icon: typeof Sun }[] = [
    { value: 'dark', label: '深色', icon: Moon },
    { value: 'light', label: '浅色', icon: Sun },
    { value: 'system', label: '跟随系统', icon: Monitor },
  ];

  return (
    <SettingsGroup label="外观">
      <SettingsRow label="主题" subtitle="界面配色，跟随系统时随系统设置自动切换">
        <div className="flex gap-1 rounded-lg bg-[var(--input-bg)] p-1">
          {themeOptions.map(({ value, label, icon: Icon }) => (
            <button
              key={value}
              onClick={() => {
                setTheme(value);
                setThemeState(value);
              }}
              className={cn(
                'flex h-7 items-center gap-1.5 rounded-md px-3 text-[13px] font-medium transition-all',
                theme === value
                  ? 'bg-[var(--panel-bg)] text-[var(--text-primary)] shadow-[var(--shadow-sm)]'
                  : 'text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]',
              )}
            >
              <Icon className="size-3.5" strokeWidth={2} />
              {label}
            </button>
          ))}
        </div>
      </SettingsRow>
      <SettingsRow label="主题色" subtitle="强调色；托盘与界面图标同步变色">
        <div className="flex items-center gap-2">
          {ACCENTS.map((a) => (
            <button
              key={a.id}
              onClick={() => {
                setAccent(a);
                setAccentId(a.id);
                // 托盘图标同步色相（浏览器 dev 环境无 Tauri，静默失败）
                void api.setTrayHue(a.hue).catch(() => {});
              }}
              data-tip={a.name}
              className={cn(
                'size-5 rounded-full transition-all duration-150',
                accentId === a.id
                  ? 'scale-110 shadow-[0_0_0_2px_var(--panel-bg),0_0_0_4px_var(--brand)]'
                  : 'hover:scale-110',
              )}
              style={{ backgroundColor: `color-mix(in srgb, ${a.light} 82%, ${a.dark})` }}
            />
          ))}
        </div>
      </SettingsRow>
      <SettingsRow label="自定义鼠标指针" subtitle="品牌色箭头/圆环光标，关闭后使用系统指针">
        <Switch checked={cursorFx} onChange={toggleCursorFx} label="自定义鼠标指针" />
      </SettingsRow>
    </SettingsGroup>
  );
}

function AutostartRow() {
  const [on, setOn] = useState(false);
  useEffect(() => {
    void import('@tauri-apps/plugin-autostart').then(async (m) => {
      setOn(await m.isEnabled());
    });
  }, []);

  const toggle = () => {
    void import('@tauri-apps/plugin-autostart').then(async (m) => {
      if (on) await m.disable();
      else await m.enable();
      setOn(!on);
    });
  };

  return (
    <SettingsRow label="开机启动" subtitle="系统登录时自动运行">
      <Switch checked={on} onChange={toggle} label="开机启动" />
    </SettingsRow>
  );
}

/** 数据目录：显示当前位置、打开、更改（重启生效） */
function DataDirRow() {
  const [info, setInfo] = useState<import('../lib/commands').DataDirInfo | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = () =>
    void api
      .getDataDir()
      .then(setInfo)
      .catch((e: Error) => setNotice(`数据目录读取失败：${e.message || e}`));
  useEffect(load, []);

  const change = async () => {
    const picked = await api.pickDataDir();
    if (!picked) return;
    try {
      await api.setDataDir(picked);
      setInfo({ current: picked, custom: true });
      setNotice('数据目录已设置，重启危信后生效（旧数据需手动迁移）');
    } catch (e) {
      setNotice(e instanceof Error ? e.message : '设置失败');
    }
  };

  return (
    <SettingsRow
      label="数据保存目录"
      subtitle={info ? (info.custom ? `${info.current}（自定义）` : info.current) : '读取中…'}
    >
      <div className="flex gap-1.5">
        <IconButton
          variant="ghost"
          size="sm"
          data-tip="打开目录"
          onClick={() => void api.openDataDir().catch((e: Error) => setNotice(`打开失败：${e.message || e}`))}
        >
          <FolderOpen className="size-4" />
        </IconButton>
        <IconButton variant="ghost" size="sm" data-tip="更改数据目录" onClick={() => void change()}>
          <FolderCog className="size-4" />
        </IconButton>
      </div>
      {notice && <p className="text-xs text-[var(--warning)]">{notice}</p>}
    </SettingsRow>
  );
}

/* ── 微信防护：目标程序下拉 + 画像 ── */

/** 目标程序行：内置应用下拉（不允许输入），替代 SchemaField 的自由文本框 */
function TargetAppRow({
  value,
  onChange,
}: {
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const current = String(value ?? '');
  return (
    <SettingsRow label="防护应用" subtitle="当前仅支持微信；记事本可用于验证防护是否生效">
      <Combobox value={current} options={TARGET_APPS} onChange={(v) => onChange(v)} className="w-44" />
    </SettingsRow>
  );
}

function ContactsGroup() {
  const [contacts, setContacts] = useState<import('../lib/types').ContactDto[]>([]);
  const [name, setName] = useState('');
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    void api.listContacts().then(setContacts);
  }, []);

  const setProfile = (n: string, p: string) => {
    setSaving(n);
    void api
      .setContactProfile(n, p)
      .then(() => api.listContacts())
      .then(setContacts)
      .finally(() => setSaving(null));
  };

  const addContact = () => {
    if (!name.trim()) return;
    setProfile(name.trim(), 'casual');
    setName('');
  };

  return (
    <SettingsGroup label="聊天对象画像">
      <SettingsRow
        label="新增对象"
        subtitle="未标记的对象一律按「正式」保守处理；画像由 OCR 识别到的对象名匹配"
        stacked
      >
        <div className="flex gap-2">
          <input
            className="h-9 min-w-0 flex-1 rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 placeholder:text-[var(--text-tertiary)] hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]"
            value={name}
            placeholder="对象名（与聊天窗口显示名一致）"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && addContact()}
          />
          <button
            className="shrink-0 rounded-lg bg-[var(--brand)] px-4 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
            disabled={!name.trim()}
            onClick={addContact}
          >
            标记为随意
          </button>
        </div>
      </SettingsRow>
      {contacts.map((c) => (
        <SettingsRow key={c.name} label={c.name}>
          <div className="flex gap-1.5">
            {(['formal', 'casual'] as const).map((p) => (
              <button
                key={p}
                disabled={saving === c.name}
                onClick={() => setProfile(c.name, c.profile === p ? 'none' : p)}
                className={cn(
                  'h-8 rounded-lg px-3.5 text-[13px] font-medium transition-all disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
                  c.profile === p
                    ? 'bg-[var(--brand)] text-[var(--brand-text)] shadow-[var(--shadow-sm)]'
                    : 'bg-[var(--input-bg)] text-[var(--text-secondary)] hover:bg-[var(--active-overlay)] hover:text-[var(--text-primary)]',
                )}
              >
                {p === 'formal' ? '正式' : '随意'}
              </button>
            ))}
          </div>
        </SettingsRow>
      ))}
    </SettingsGroup>
  );
}

/* ── 违禁词库 ── */

/** 违禁词编辑器（即时保存，移除显式保存按钮） */
function RulesEditor() {
  const [rules, setRules] = useState<import('../lib/types').RuleDto[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const saveTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    void api.getRules().then(setRules);
  }, []);

  const save = (updatedRules: import('../lib/types').RuleDto[]) => {
    if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    saveTimeoutRef.current = window.setTimeout(() => {
      setSaving(true);
      setError(null);
      void api
        .saveRules(updatedRules)
        .then(() => setSaving(false))
        .catch((e: Error) => {
          setError(e.message || '保存失败');
          setSaving(false);
        });
    }, 600);
  };

  const update = (i: number, patch: Partial<import('../lib/types').RuleDto>) => {
    const updated = rules.map((r, j) => (j === i ? { ...r, ...patch } : r));
    setRules(updated);
    save(updated);
  };

  const add = () => {
    const updated = [
      ...rules,
      { pattern: '', match: 'substring', severity: 'warn', applies_to: ['all'] },
    ];
    setRules(updated);
  };

  const remove = (i: number) => {
    const updated = rules.filter((_, j) => j !== i);
    setRules(updated);
    save(updated);
  };

  const inputCls =
    'h-9 min-w-0 rounded-lg bg-[var(--input-bg)] px-3 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]';

  return (
    <div className="mx-auto max-w-3xl">
      <div className="mb-5 flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-[var(--text-primary)]">违禁词库</h2>
          <p className="mt-1 text-[13px] text-[var(--text-tertiary)]">
            修改后自动保存 {saving && <span className="text-[var(--brand)]">· 保存中...</span>}
          </p>
        </div>
        <button
          className="flex items-center gap-1.5 rounded-lg bg-[var(--brand)] px-3.5 py-2 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
          onClick={add}
        >
          <Plus className="size-4" />
          添加规则
        </button>
      </div>

      {error && (
        <div className="mb-4 rounded-lg bg-[var(--danger-soft)] px-4 py-2 text-sm text-[var(--danger)]">
          {error}
        </div>
      )}

      <div className="space-y-2">
        {rules.map((r, i) => (
          <div key={i} className="flex items-center gap-2 rounded-xl bg-[var(--group-bg)] px-3 py-2">
            <input
              className={cn(inputCls, 'flex-1')}
              value={r.pattern}
              placeholder="词/正则"
              onChange={(e) => update(i, { pattern: e.target.value })}
            />
            <select
              className={cn(inputCls, 'w-24 cursor-pointer')}
              value={r.match}
              onChange={(e) => update(i, { match: e.target.value })}
            >
              <option value="word">整词</option>
              <option value="substring">包含</option>
              <option value="regex">正则</option>
            </select>
            <select
              className={cn(inputCls, 'w-24 cursor-pointer')}
              value={r.severity}
              onChange={(e) => update(i, { severity: e.target.value })}
            >
              <option value="warn">警告</option>
              <option value="block">阻断</option>
            </select>
            <select
              className={cn(inputCls, 'w-28 cursor-pointer')}
              value={r.applies_to[0] ?? 'all'}
              onChange={(e) => update(i, { applies_to: [e.target.value] })}
            >
              <option value="all">全部场景</option>
              <option value="formal">正式</option>
              <option value="casual">随意</option>
            </select>
            <IconButton variant="ghost" onClick={() => remove(i)} data-tip="删除规则">
              <Trash2 className="size-4 text-[var(--danger)]" />
            </IconButton>
          </div>
        ))}
        {rules.length === 0 && (
          <p className="py-12 text-center text-sm text-[var(--text-tertiary)]">
            暂无规则，点击「添加规则」创建
          </p>
        )}
      </div>
    </div>
  );
}

/* ── 高级设置：日志 ── */

function LogsGroup() {
  const [clearing, setClearing] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const clearLogs = () => {
    setClearing(true);
    void api
      .clearLogs()
      .then(() => {
        setConfirming(false);
        setClearing(false);
      })
      .catch(() => setClearing(false));
  };

  return (
    <SettingsGroup label="日志">
      <SettingsRow label="清空全部日志" subtitle="删除日志目录全部内容（不可恢复）">
        {confirming ? (
          <div className="flex gap-2">
            <button
              className="rounded-lg bg-[var(--danger)] px-3.5 py-1.5 text-[13px] font-medium text-white shadow-[var(--shadow-sm)] transition-opacity hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
              disabled={clearing}
              onClick={clearLogs}
            >
              {clearing ? '清空中...' : '确认清空'}
            </button>
            <button
              className="rounded-lg bg-[var(--input-bg)] px-3.5 py-1.5 text-[13px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--active-overlay)] hover:text-[var(--text-primary)]"
              onClick={() => setConfirming(false)}
            >
              取消
            </button>
          </div>
        ) : (
          <button
            className="rounded-lg bg-[var(--input-bg)] px-3.5 py-1.5 text-[13px] font-medium text-[var(--text-primary)] transition-colors hover:bg-[var(--danger-soft)] hover:text-[var(--danger)]"
            onClick={() => setConfirming(true)}
          >
            清空全部日志
          </button>
        )}
      </SettingsRow>
    </SettingsGroup>
  );
}

/* ── 关于 ── */

function AboutPage() {
  return (
    <div className="mx-auto max-w-2xl">
      <AboutHero name="危信" version="0.1.0" />
      <SettingsSection>
        <SettingsGroup label="软件简介">
          <SettingsRow label="做什么" stacked>
            <p className="text-sm leading-relaxed text-[var(--text-secondary)]">
              危险言语提前拦截工具。截取屏幕画面在本机识别文字，在消息发出前提醒你。
            </p>
          </SettingsRow>
          <SettingsRow label="不做什么" stacked>
            <ul className="space-y-1 text-sm text-[var(--text-secondary)]">
              <li>· 不注入或修改微信程序</li>
              <li>· 不读取或解密微信聊天记录文件</li>
              <li>· 不替你发送任何消息</li>
              <li>· 不把聊天内容上传到任何服务器（纯本地方案）</li>
            </ul>
          </SettingsRow>
          <SettingsRow label="自行验证" stacked>
            <ul className="space-y-1 text-sm text-[var(--text-secondary)]">
              <li>· 用 Wireshark 抓包——默认配置下不发起任何网络连接</li>
              <li>· 用 Process Monitor 观察——从不读取微信目录、不访问微信进程内存</li>
              <li>· 源码完全公开，可自行审计与构建</li>
            </ul>
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
    </div>
  );
}
