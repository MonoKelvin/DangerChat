import { useEffect, useMemo, useState } from 'react';
import { cn } from '../lib/utils';
import type { ConfigFieldDto, StatusPayload } from '../lib/types';
import * as api from '../lib/commands';
import { onStats, onStatus } from '../lib/events';
import { SchemaField } from './SchemaField';

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
  const [saved, setSaved] = useState<string | null>(null);

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
    void api.setConfig(key, v).then(() => {
      setSaved(key);
      setTimeout(() => setSaved(null), 1200);
    });
  };

  const stateColor =
    status?.state === 'active' && status.target_found
      ? 'bg-emerald-500'
      : status?.state === 'paused'
        ? 'bg-amber-500'
        : 'bg-slate-400';

  const stateText =
    status == null
      ? '…'
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
      {/* 左侧分类导航（FR-UI-02） */}
      <nav className="flex w-48 shrink-0 flex-col border-r border-border/60 bg-sidebar/50 p-2">
        <div className="mb-2 flex items-center gap-2.5 px-2 pt-1 pb-3">
          <div className="flex size-7 items-center justify-center rounded-md bg-gradient-to-br from-orange-500 to-rose-500 text-xs font-bold text-white">
            危
          </div>
          <div className="flex flex-col leading-tight">
            <span className="text-[13px] font-semibold">危信</span>
            <span className="mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground">
              <span className={cn('size-1.5 rounded-full', stateColor)} />
              {stateText}
            </span>
          </div>
        </div>
        {CATEGORY_ORDER.map((c) => (
          <button
            key={c}
            onClick={() => setActive(c)}
            className={cn(
              'rounded-md px-3 py-2 text-left text-[13px] transition-colors',
              active === c
                ? 'bg-accent font-medium text-accent-foreground'
                : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
            )}
          >
            {c}
          </button>
        ))}
        {stats && (
          <div className="mt-auto px-3 pb-1 text-[11px] text-muted-foreground">
            今日拦截 <b className="tabular-nums text-foreground">{stats.today_blocked}</b> 次
          </div>
        )}
      </nav>

      {/* 右侧设置区 */}
      <div className="min-w-0 flex-1 overflow-y-auto p-5">
        {/* 通用/目标/拦截/模型/隐私：schema 驱动 */}
        {!SPECIAL.includes(active as (typeof SPECIAL)[number]) && (
          <div className="mx-auto max-w-2xl space-y-3">
            <h2 className="mb-4 text-base font-semibold">{active}</h2>
            {(byCategory.get(active) ?? []).map((f) => (
              <div key={f.key} className="relative">
                <SchemaField
                  field={f}
                  value={values[f.key] ?? f.default}
                  onChange={(v) => change(f.key, v)}
                />
                {saved === f.key && (
                  <span className="absolute top-3 right-4 text-[11px] text-emerald-500">已保存</span>
                )}
              </div>
            ))}
            {(byCategory.get(active) ?? []).length === 0 && (
              <p className="py-8 text-center text-sm text-muted-foreground">此分类暂无配置项</p>
            )}
            {active === '通用' && (
              <div className="flex items-center justify-between rounded-lg border border-border/40 bg-card/50 px-4 py-3">
                <div>
                  <p className="text-sm font-medium">开机启动</p>
                  <p className="mt-0.5 text-xs text-muted-foreground">系统登录时自动运行（独立于静默启动）</p>
                </div>
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
  return (
    <button
      role="switch"
      aria-checked={on}
      onClick={() => {
        void import('@tauri-apps/plugin-autostart').then(async (m) => {
          if (on) await m.disable();
          else await m.enable();
          setOn(!on);
        });
      }}
      className={cn(
        'relative h-6 w-11 shrink-0 rounded-full transition-colors',
        on ? 'bg-primary' : 'bg-muted-foreground/30',
      )}
    >
      <span
        className={cn(
          'absolute top-0.5 size-5 rounded-full bg-white shadow transition-all',
          on ? 'left-[22px]' : 'left-0.5',
        )}
      />
    </button>
  );
}

/** 违禁词编辑器（UT-UI-04：增删改 + 保存往返） */
function RulesEditor() {
  const [rules, setRules] = useState<import('../lib/types').RuleDto[]>([]);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    void api.getRules().then(setRules);
  }, []);

  const update = (i: number, patch: Partial<import('../lib/types').RuleDto>) => {
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
    setDirty(true);
  };
  const add = () => {
    setRules((rs) => [
      ...rs,
      { pattern: '', match: 'substring', severity: 'warn', applies_to: ['all'] },
    ]);
    setDirty(true);
  };
  const remove = (i: number) => {
    setRules((rs) => rs.filter((_, j) => j !== i));
    setDirty(true);
  };

  return (
    <div className="mx-auto max-w-3xl">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-base font-semibold">违禁词库</h2>
        <div className="flex gap-2">
          <button
            className="h-8 rounded-md border border-border/60 px-3 text-xs hover:bg-accent"
            onClick={add}
          >
            添加规则
          </button>
          <button
            className="h-8 rounded-md bg-primary px-3 text-xs text-primary-foreground disabled:opacity-40"
            disabled={!dirty}
            onClick={() => {
              void api.saveRules(rules).then(() => setDirty(false));
            }}
          >
            {dirty ? '保存' : '已保存'}
          </button>
        </div>
      </div>
      <div className="space-y-2">
        {rules.map((r, i) => (
          <div key={i} className="flex items-center gap-2 rounded-lg border border-border/40 bg-card/50 px-3 py-2">
            <input
              className="h-8 min-w-0 flex-1 rounded-md border border-border/60 bg-background px-2.5 text-sm"
              value={r.pattern}
              placeholder="词/正则"
              onChange={(e) => update(i, { pattern: e.target.value })}
            />
            <select
              className="h-8 rounded-md border border-border/60 bg-background px-2 text-xs"
              value={r.match}
              onChange={(e) => update(i, { match: e.target.value })}
            >
              <option value="word">整词</option>
              <option value="substring">包含</option>
              <option value="regex">正则</option>
            </select>
            <select
              className="h-8 rounded-md border border-border/60 bg-background px-2 text-xs"
              value={r.severity}
              onChange={(e) => update(i, { severity: e.target.value })}
            >
              <option value="warn">警告</option>
              <option value="block">阻断</option>
            </select>
            <select
              className="h-8 rounded-md border border-border/60 bg-background px-2 text-xs"
              value={r.applies_to[0] ?? 'all'}
              onChange={(e) => update(i, { applies_to: [e.target.value] })}
            >
              <option value="all">全部场景</option>
              <option value="formal">正式</option>
              <option value="casual">随意</option>
            </select>
            <button
              className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
              onClick={() => remove(i)}
              aria-label="删除"
            >
              ×
            </button>
          </div>
        ))}
        {rules.length === 0 && (
          <p className="py-8 text-center text-sm text-muted-foreground">暂无规则，点「添加规则」创建</p>
        )}
      </div>
    </div>
  );
}

function ContactsEditor() {
  const [contacts, setContacts] = useState<import('../lib/types').ContactDto[]>([]);
  const [name, setName] = useState('');

  useEffect(() => {
    void api.listContacts().then(setContacts);
  }, []);

  const setProfile = (n: string, p: string) => {
    void api.setContactProfile(n, p).then(() => {
      void api.listContacts().then(setContacts);
    });
  };

  return (
    <div className="mx-auto max-w-2xl">
      <h2 className="mb-1 text-base font-semibold">聊天对象画像</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        未标记的对象一律按「正式」保守处理。画像由 OCR 识别到的对象名匹配（每次慢环更新）。
      </p>
      <div className="mb-4 flex gap-2">
        <input
          className="h-9 min-w-0 flex-1 rounded-md border border-border/60 bg-background px-3 text-sm"
          value={name}
          placeholder="对象名（与聊天窗口显示名一致）"
          onChange={(e) => setName(e.target.value)}
        />
        <button
          className="h-9 rounded-md bg-primary px-4 text-xs text-primary-foreground disabled:opacity-40"
          disabled={!name.trim()}
          onClick={() => {
            setProfile(name.trim(), 'casual');
            setName('');
          }}
        >
          标记为随意
        </button>
      </div>
      <div className="space-y-2">
        {contacts.map((c) => (
          <div
            key={c.name}
            className="flex items-center justify-between rounded-lg border border-border/40 bg-card/50 px-4 py-2.5"
          >
            <span className="text-sm">{c.name}</span>
            <div className="flex gap-1">
              {(['formal', 'casual'] as const).map((p) => (
                <button
                  key={p}
                  onClick={() => setProfile(c.name, c.profile === p ? 'none' : p)}
                  className={cn(
                    'h-7 rounded-md px-3 text-xs transition-colors',
                    c.profile === p
                      ? 'bg-primary text-primary-foreground'
                      : 'border border-border/60 text-muted-foreground hover:bg-accent',
                  )}
                >
                  {p === 'formal' ? '正式' : '随意'}
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function AboutPage() {
  return (
    <div className="mx-auto max-w-2xl space-y-4">
      <h2 className="text-base font-semibold">关于</h2>
      <div className="rounded-lg border border-border/40 bg-card/50 p-4 text-sm leading-relaxed">
        <p className="font-medium">危信 v0.1.0</p>
        <p className="mt-2 text-muted-foreground">
          危险言语提前拦截工具。纯本地运行：屏幕像素识别 + 本地模型，不联网、不读写微信文件、
          不代发任何消息。开源可审计。
        </p>
      </div>
      <div className="rounded-lg border border-border/40 bg-card/50 p-4">
        <button
          className="h-8 rounded-md border border-border/60 px-3 text-xs hover:bg-accent"
          onClick={() => void api.clearLogs()}
        >
          清空全部日志
        </button>
        <p className="mt-2 text-xs text-muted-foreground">删除日志目录全部内容（FR-SYS-05）</p>
      </div>
    </div>
  );
}
