import { useEffect, useMemo, useState, useRef } from 'react';
import { Shield, ShieldAlert, ShieldOff, Plus, Trash2 } from 'lucide-react';
import { cn } from '../lib/utils';
import type { ConfigFieldDto, StatusPayload } from '../lib/types';
import * as api from '../lib/commands';
import { onStats, onStatus } from '../lib/events';
import { SchemaField } from './SchemaField';
import { Switch } from '../components/Switch';
import { IconButton } from '../components/IconButton';

/** 分类导航（FR-UI-03）。schema group → 分类；违禁词库/画像/关于是专用页。 */
const CATEGORY_ORDER = [
  '通用',
  '目标程序',
  '拦截与提示',
  '违禁词库',
  '聊天对象画像',
  '模型与设备',
  '日志与隐私',
  '关于',
] as const;

const SPECIAL = ['违禁词库', '聊天对象画像', '关于'] as const;
type Category = (typeof CATEGORY_ORDER)[number];

/** schema group 名 → 分类名（模块 group 是英文短名，映射到界面分类）。 */
function groupToCategory(group: string): Category {
  const map: Record<string, Category> = {
    general: '通用',
    target: '目标程序',
    guard: '拦截与提示',
    '拦截与提示': '拦截与提示',
    '模型与设备': '模型与设备',
    privacy: '日志与隐私',
    '日志与隐私': '日志与隐私',
    alert: '拦截与提示',
    pipeline: '拦截与提示',
    device: '模型与设备',
  };
  return map[group] ?? '通用';
}

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

  const byCategory = useMemo(() => {
    const m = new Map<Category, ConfigFieldDto[]>();
    for (const f of fields) {
      const c = groupToCategory(f.group);
      m.set(c, [...(m.get(c) ?? []), f]);
    }
    return m;
  }, [fields]);

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

  return (
    <div className="flex h-full">
      {/* 左侧分类导航 */}
      <nav className="flex w-56 shrink-0 flex-col gap-1.5 bg-[var(--sidebar-bg)] px-4 py-6">
        <div className="mb-6 flex items-center gap-3 px-1">
          <div className="relative flex size-10 items-center justify-center rounded-xl bg-gradient-to-br from-purple-500 via-purple-600 to-indigo-600 text-base font-bold text-white shadow-lg">
            <span className="relative z-10">危</span>
            <div className="absolute inset-0 rounded-xl bg-gradient-to-br from-purple-400 to-indigo-500 opacity-0 blur-md transition-opacity group-hover:opacity-60" />
          </div>
          <div className="flex flex-1 flex-col leading-tight">
            <span className="text-base font-semibold tracking-tight text-[var(--text-primary)]">危信</span>
            <div className="mt-1 flex items-center gap-1.5 text-xs">
              <StateIcon className={cn('size-3.5', stateColor)} strokeWidth={2} />
              <span className="text-[var(--text-secondary)]">{stateText}</span>
            </div>
          </div>
        </div>

        <div className="space-y-0.5">
          {CATEGORY_ORDER.map((c) => (
            <button
              key={c}
              onClick={() => setActive(c)}
              className={cn(
                'w-full rounded-lg px-3 py-2.5 text-left text-[13px] font-medium transition-all duration-150',
                active === c
                  ? 'bg-[var(--primary)] text-[var(--primary-text)] shadow-[var(--shadow-md)]'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
              )}
            >
              {c}
            </button>
          ))}
        </div>

        {stats && (
          <div className="mt-auto rounded-lg bg-[var(--card-bg)] px-3.5 py-3 shadow-[var(--shadow-sm)]">
            <div className="flex items-baseline gap-1.5">
              <span className="text-xs text-[var(--text-secondary)]">今日拦截</span>
              <strong className="text-lg font-semibold tabular-nums tracking-tight text-[var(--primary)]">
                {stats.today_blocked}
              </strong>
              <span className="text-xs text-[var(--text-tertiary)]">次</span>
            </div>
          </div>
        )}
      </nav>

      {/* 右侧设置区 */}
      <div className="min-w-0 flex-1 overflow-y-auto bg-[var(--panel-bg)] px-8 py-6">
        {!SPECIAL.includes(active as (typeof SPECIAL)[number]) && (
          <div className="mx-auto max-w-2xl space-y-4">
            <h2 className="mb-6 text-xl font-semibold tracking-tight text-[var(--text-primary)]">{active}</h2>
            {(byCategory.get(active) ?? []).map((f) => (
              <SchemaField
                key={f.key}
                field={f}
                value={values[f.key] ?? f.default}
                onChange={(v) => change(f.key, v)}
              />
            ))}
            {(byCategory.get(active) ?? []).length === 0 && (
              <p className="py-16 text-center text-sm text-[var(--text-tertiary)]">此分类暂无配置项</p>
            )}
            {active === '通用' && (
              <div className="mt-6">
                <AutostartToggle />
              </div>
            )}
          </div>
        )}

        {active === '违禁词库' && <RulesEditor />}
        {active === '聊天对象画像' && <ContactsEditor />}
        {active === '关于' && <AboutPage />}
      </div>
    </div>
  );
}

function AutostartToggle() {
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
    <div className="flex items-center justify-between rounded-lg bg-[var(--card-bg)] px-5 py-4 shadow-[var(--shadow-sm)] transition-all duration-150 hover:shadow-[var(--shadow-md)]">
      <div>
        <p className="text-[13px] font-medium leading-snug text-[var(--text-primary)]">开机启动</p>
        <p className="mt-1 text-xs text-[var(--text-secondary)]">系统登录时自动运行</p>
      </div>
      <Switch checked={on} onChange={toggle} label="开机启动" />
    </div>
  );
}

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

  return (
    <div className="mx-auto max-w-3xl">
      <div className="mb-5 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-[var(--text-primary)]">违禁词库</h2>
          <p className="mt-1 text-xs text-[var(--text-secondary)]">
            修改后自动保存 {saving && <span className="text-[var(--primary)]">· 保存中...</span>}
          </p>
        </div>
        <button
          className="flex items-center gap-1.5 rounded-lg bg-[var(--primary)] px-3 py-2 text-sm font-medium text-[var(--primary-text)] shadow-sm transition-all hover:bg-[var(--primary-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
          onClick={add}
        >
          <Plus className="size-4" />
          添加规则
        </button>
      </div>

      {error && (
        <div className="mb-4 rounded-lg bg-[var(--danger)]/10 px-4 py-2 text-sm text-[var(--danger)]">
          {error}
        </div>
      )}

      <div className="space-y-2.5">
        {rules.map((r, i) => (
          <div
            key={i}
            className="flex items-center gap-2.5 rounded-lg bg-[var(--card-bg)] px-3 py-2.5 shadow-[var(--shadow-sm)] transition-shadow hover:shadow-[var(--shadow-md)]"
          >
            <input
              className="h-9 min-w-0 flex-1 rounded-md border border-[var(--border)] bg-[var(--input-bg)] px-3 text-sm text-[var(--text-primary)] transition-colors focus:border-[var(--primary)] focus:outline-none focus:ring-2 focus:ring-[var(--focus-ring)]"
              value={r.pattern}
              placeholder="词/正则"
              onChange={(e) => update(i, { pattern: e.target.value })}
            />
            <select
              className="h-9 rounded-md border border-[var(--border)] bg-[var(--input-bg)] px-2.5 text-xs text-[var(--text-primary)] transition-colors focus:border-[var(--primary)] focus:outline-none focus:ring-2 focus:ring-[var(--focus-ring)]"
              value={r.match}
              onChange={(e) => update(i, { match: e.target.value })}
            >
              <option value="word">整词</option>
              <option value="substring">包含</option>
              <option value="regex">正则</option>
            </select>
            <select
              className="h-9 rounded-md border border-[var(--border)] bg-[var(--input-bg)] px-2.5 text-xs text-[var(--text-primary)] transition-colors focus:border-[var(--primary)] focus:outline-none focus:ring-2 focus:ring-[var(--focus-ring)]"
              value={r.severity}
              onChange={(e) => update(i, { severity: e.target.value })}
            >
              <option value="warn">警告</option>
              <option value="block">阻断</option>
            </select>
            <select
              className="h-9 rounded-md border border-[var(--border)] bg-[var(--input-bg)] px-2.5 text-xs text-[var(--text-primary)] transition-colors focus:border-[var(--primary)] focus:outline-none focus:ring-2 focus:ring-[var(--focus-ring)]"
              value={r.applies_to[0] ?? 'all'}
              onChange={(e) => update(i, { applies_to: [e.target.value] })}
            >
              <option value="all">全部场景</option>
              <option value="formal">正式</option>
              <option value="casual">随意</option>
            </select>
            <IconButton
              variant="ghost"
              onClick={() => remove(i)}
              title="删除规则"
            >
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

function ContactsEditor() {
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
    <div className="mx-auto max-w-2xl">
      <h2 className="mb-1 text-lg font-semibold text-[var(--text-primary)]">聊天对象画像</h2>
      <p className="mb-5 text-xs text-[var(--text-secondary)]">
        未标记的对象一律按「正式」保守处理。画像由 OCR 识别到的对象名匹配。
      </p>

      <div className="mb-5 flex gap-2.5">
        <input
          className="h-10 min-w-0 flex-1 rounded-lg border border-[var(--border)] bg-[var(--input-bg)] px-3 text-sm text-[var(--text-primary)] transition-colors focus:border-[var(--primary)] focus:outline-none focus:ring-2 focus:ring-[var(--focus-ring)]"
          value={name}
          placeholder="对象名（与聊天窗口显示名一致）"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && addContact()}
        />
        <button
          className="flex items-center gap-1.5 rounded-lg bg-[var(--primary)] px-4 py-2 text-sm font-medium text-[var(--primary-text)] shadow-sm transition-all hover:bg-[var(--primary-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
          disabled={!name.trim()}
          onClick={addContact}
        >
          标记为随意
        </button>
      </div>

      <div className="space-y-2.5">
        {contacts.map((c) => (
          <div
            key={c.name}
            className="flex items-center justify-between rounded-lg bg-[var(--card-bg)] px-4 py-3 shadow-[var(--shadow-sm)] transition-shadow hover:shadow-[var(--shadow-md)]"
          >
            <span className="text-sm font-medium text-[var(--text-primary)]">{c.name}</span>
            <div className="flex gap-2">
              {(['formal', 'casual'] as const).map((p) => (
                <button
                  key={p}
                  disabled={saving === c.name}
                  onClick={() => setProfile(c.name, c.profile === p ? 'none' : p)}
                  className={cn(
                    'h-8 rounded-md px-3 text-xs font-medium transition-all disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]',
                    c.profile === p
                      ? 'bg-[var(--primary)] text-[var(--primary-text)] shadow-sm'
                      : 'border border-[var(--border)] text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
                  )}
                >
                  {p === 'formal' ? '正式' : '随意'}
                </button>
              ))}
            </div>
          </div>
        ))}
        {contacts.length === 0 && (
          <p className="py-12 text-center text-sm text-[var(--text-tertiary)]">
            暂无画像，输入对象名后点击「标记为随意」创建
          </p>
        )}
      </div>
    </div>
  );
}

function AboutPage() {
  const [clearing, setClearing] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);

  const clearLogs = () => {
    setClearing(true);
    void api
      .clearLogs()
      .then(() => {
        setShowConfirm(false);
        setClearing(false);
      })
      .catch(() => setClearing(false));
  };

  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <h2 className="text-lg font-semibold text-[var(--text-primary)]">关于</h2>

      <div className="rounded-lg bg-[var(--card-bg)] p-5 shadow-[var(--shadow-sm)]">
        <p className="text-sm font-semibold text-[var(--text-primary)]">危信 v0.1.0</p>
        <p className="mt-2.5 text-sm leading-relaxed text-[var(--text-secondary)]">
          危险言语提前拦截工具。纯本地运行：屏幕像素识别 + 本地模型，不联网、不读写微信文件、
          不代发任何消息。开源可审计。
        </p>
      </div>

      <div className="rounded-lg bg-[var(--card-bg)] p-5 shadow-[var(--shadow-sm)]">
        <p className="mb-2 text-sm font-medium text-[var(--text-primary)]">数据管理</p>
        {!showConfirm ? (
          <>
            <button
              className="rounded-lg border border-[var(--border)] px-3 py-2 text-xs text-[var(--text-primary)] transition-colors hover:bg-[var(--hover-overlay)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
              onClick={() => setShowConfirm(true)}
            >
              清空全部日志
            </button>
            <p className="mt-2 text-xs text-[var(--text-secondary)]">
              删除日志目录全部内容（不可恢复）
            </p>
          </>
        ) : (
          <div className="flex items-center gap-2.5">
            <p className="text-xs text-[var(--danger)]">确定要清空全部日志吗？此操作不可恢复。</p>
            <button
              className="rounded-lg bg-[var(--danger)] px-3 py-2 text-xs font-medium text-white shadow-sm transition-opacity hover:opacity-90 disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
              disabled={clearing}
              onClick={clearLogs}
            >
              {clearing ? '清空中...' : '确认清空'}
            </button>
            <button
              className="rounded-lg border border-[var(--border)] px-3 py-2 text-xs text-[var(--text-primary)] transition-colors hover:bg-[var(--hover-overlay)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
              onClick={() => setShowConfirm(false)}
            >
              取消
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
