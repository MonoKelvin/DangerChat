import { useEffect, useMemo, useState, useRef, type ReactNode } from 'react';
import { Shield, ShieldAlert, ShieldOff, Plus, Trash2, Pencil, FolderOpen, FolderCog, Loader2, Moon, Sun, Monitor, GraduationCap } from 'lucide-react';
import { cn } from '../lib/utils';
import type { ConfigFieldDto, ModelDto, ScenarioDto, StatusPayload } from '../lib/types';
import * as api from '../lib/commands';
import { onStats, onStatus, onModels } from '../lib/events';
import { SchemaField } from './SchemaField';
import { Switch } from '../components/Switch';
import { Checkbox } from '../components/Checkbox';
import { Modal, ModalButton } from '../components/Modal';
import { IconButton } from '../components/IconButton';
import { Combobox, type ComboOption } from '../components/Combobox';
import { SettingsGroup, SettingsRow, SettingsSection } from '../components/SettingsGroup';
import { AboutHero } from './AboutHero';
import { TrainingDialog } from './TrainingDialog';
import { APP_NAME, APP_VERSION, AUTHOR, AUTHOR_URL, LICENSE, REPO_URL } from '../lib/meta';
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

/** 分类导航（FR-UI-03）：按用户视角归组——日常防护开关 + 场景与词库 +
 *  面向开发者的技术参数（高级设置）。 */
const CATEGORY_ORDER = ['通用', '消息防护', '场景管理', '高级设置', '关于'] as const;
type Category = (typeof CATEGORY_ORDER)[number];

/** 技术参数键（判定有效期/防抖等一般人无需关心的）→ 高级设置 */
const ADVANCED_KEYS = new Set([
  'guard.verdict_ttl_ms',
  'guard.foreground_debounce_ms',
]);

/** 内置目标程序（下拉选择，不允许输入） */
const TARGET_APPS: ComboOption[] = [
  { value: 'Weixin.exe', label: '微信' },
  { value: 'notepad.exe', label: '记事本', hint: '验证用' },
];

/** 消息防护页的分组顺序与标题 */
const WECHAT_GROUPS: { title: string; keys: string[] }[] = [
  { title: '目标程序', keys: ['target.process_name'] },
  { title: '拦截行为', keys: ['guard.enabled', 'guard.send_key', 'alert.timeout_secs'] },
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

  /** 渲染一组 schema 字段（目标程序/区域模型特殊渲染为自定义行） */
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
    if (key === 'layout.model') {
      return (
        <LayoutModelRow
          key={key}
          value={String(values[key] ?? byKey.get(key)?.default ?? 'dc-layout-wechat')}
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
            alt={APP_NAME}
            className="size-9 rounded-xl object-cover shadow-[var(--shadow-sm)] transition-[filter] duration-300"
            style={{ filter: logoFilter }}
          />
          <div className="flex flex-1 flex-col leading-tight">
            <span className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">{APP_NAME}</span>
            <div className="mt-0.5 flex items-center gap-1.5 text-xs">
              <StateIcon className={cn('size-3.5', stateColor)} strokeWidth={2} />
              <span className="text-[var(--text-tertiary)]">{stateText}</span>
            </div>
          </div>
        </div>

        <div className="space-y-1.5">
          {CATEGORY_ORDER.map((c) => (
            <button
              key={c}
              onClick={() => setActive(c)}
              className={cn(
                'w-full rounded-lg px-3 py-2.5 text-left text-sm font-medium transition-all duration-150',
                active === c
                  ? 'bg-[var(--brand)] text-[var(--brand-text)] shadow-[var(--shadow-sm)]'
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
        {active === '消息防护' && (
          <div className="mx-auto max-w-2xl">
            <h2 className="mb-5 text-xl font-semibold tracking-tight text-[var(--text-primary)]">消息防护</h2>
            <SettingsSection>
              {WECHAT_GROUPS.slice(0, 2).map((g) => (
                <SettingsGroup key={g.title} label={g.title}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
              <ContactsGroup />
              {WECHAT_GROUPS.slice(2).map((g) => (
                <SettingsGroup key={g.title} label={g.title}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
            </SettingsSection>
          </div>
        )}
        {active === '场景管理' && <ScenariosPage />}
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
                  ? 'bg-[var(--brand)] text-[var(--brand-text)] shadow-[var(--shadow-sm)]'
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

/** 字节数 → 人类可读（迁移回执用） */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 10 ? 0 : 1)} ${units[i]}`;
}

/** 数据目录：显示当前位置、打开、一键迁移到新位置（重启生效） */
function DataDirRow() {
  const [info, setInfo] = useState<import('../lib/commands').DataDirInfo | null>(null);
  /** 行内通知：仅承载错误（成功走弹窗，避免长文案挤压行布局） */
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** 迁移成功回执；非 null 即弹窗打开 */
  const [report, setReport] = useState<import('../lib/commands').MigrateReport | null>(null);

  const load = () =>
    void api
      .getDataDir()
      .then(setInfo)
      .catch((e: Error) => setError(`数据目录读取失败：${e.message || e}`));
  useEffect(load, []);

  /** 选目录 → 复制全部数据 → 写指针。旧目录去留由结果弹窗决定。 */
  const migrate = async () => {
    const picked = await api.pickDataDir();
    if (!picked) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.migrateDataDir(picked);
      setInfo({ current: r.target, custom: true });
      setReport(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : '迁移失败');
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsRow
      label="数据目录"
      subtitle={info ? (info.custom ? `${info.current}（自定义）` : info.current) : '读取中…'}
    >
      <div className="flex gap-1.5">
        <IconButton
          variant="ghost"
          size="sm"
          data-tip="打开目录"
          onClick={() => void api.openDataDir().catch((e: Error) => setError(`打开失败：${e.message || e}`))}
        >
          <FolderOpen className="size-4" />
        </IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          disabled={busy}
          data-tip={busy ? '迁移中…' : '一键迁移到新位置'}
          onClick={() => void migrate()}
        >
          {busy ? <Loader2 className="size-4 animate-spin" /> : <FolderCog className="size-4" />}
        </IconButton>
      </div>
      {error && <p className="text-xs text-[var(--warning)]">{error}</p>}
      {report && <MigrateResultDialog report={report} onClose={() => setReport(null)} />}
    </SettingsRow>
  );
}

/**
 * 迁移结果对话框：告知重启生效 + 询问旧目录去留。
 *
 * 三个出口（重启软件 / 稍后重启 / 关闭）**都在关闭时执行清理** —— 勾选即时生效，
 * 不依赖是否重启（清理的是已迁移走的旧目录，与当前进程运行无关）。
 */
function MigrateResultDialog({
  report,
  onClose,
}: {
  report: import('../lib/commands').MigrateReport;
  onClose: () => void;
}) {
  /** 默认勾选：迁移后旧目录通常是冗余的，让用户顺手清掉 */
  const [removeOld, setRemoveOld] = useState(true);
  const [cleanupError, setCleanupError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** 防重入：清理/重启进行中不再接受关闭（避免并发删除） */
  const closingRef = useRef(false);

  /** 依勾选清理旧目录；返回错误消息（null = 成功或无需清理） */
  const cleanupIfNeeded = async (): Promise<string | null> => {
    if (!removeOld) return null;
    try {
      await api.deleteOldDataDir(report.previous);
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : '旧目录删除失败';
    }
  };

  /** 统一关闭流程：清理 → （失败则留窗示错）/（成功则关闭） */
  const closeWithCleanup = async (after?: () => Promise<void>) => {
    if (closingRef.current) return;
    closingRef.current = true;
    setBusy(true);
    setCleanupError(null);

    const failed = await cleanupIfNeeded();
    if (failed) {
      setCleanupError(failed);
      setBusy(false);
      closingRef.current = false;
      return;
    }
    if (after) {
      try {
        await after();
      } catch (e) {
        setCleanupError(e instanceof Error ? e.message : '操作失败');
        setBusy(false);
        closingRef.current = false;
        return;
      }
    }
    onClose();
  };

  return (
    <Modal
      open
      title="数据已迁移"
      // 点遮罩 / Esc：等同「关闭」
      onClose={() => void closeWithCleanup()}
      footer={
        <>
          {busy && (
            <span className="mr-auto flex items-center gap-1.5 text-xs text-[var(--text-tertiary)]">
              <Loader2 className="size-3.5 animate-spin" />
              处理中…
            </span>
          )}
          <ModalButton
            variant="primary"
            disabled={busy}
            onClick={() => void closeWithCleanup(() => api.restartApp())}
          >
            重启软件
          </ModalButton>
          <ModalButton disabled={busy} onClick={() => void closeWithCleanup()}>
            稍后重启
          </ModalButton>
          <ModalButton disabled={busy} onClick={() => void closeWithCleanup()}>
            关闭
          </ModalButton>
        </>
      }
    >
      <p>
        已迁移 {report.files} 个文件（{formatBytes(report.bytes)}），重启软件后生效。
      </p>
      <div className="mt-3.5 border-t border-[var(--divider)] pt-3">
        <Checkbox
          checked={removeOld}
          onChange={setRemoveOld}
          label={`删除旧目录（${report.previous}）`}
        />
      </div>
      {cleanupError && (
        <p className="mt-2.5 text-xs text-[var(--warning)]">旧目录清理失败：{cleanupError}</p>
      )}
    </Modal>
  );
}

/* ── 消息防护：目标程序下拉 + 画像 ── */

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

/** 区域模型行：下拉列出 models/ 有效 layout 模型（目录监听自动刷新）+ 自助训练入口 */
function LayoutModelRow({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [models, setModels] = useState<ModelDto[]>([]);
  const [trainingOpen, setTrainingOpen] = useState(false);

  useEffect(() => {
    void api.listModels().then(setModels);
    const un = onModels(setModels);
    return () => void un.then((f) => f());
  }, []);

  const options: ComboOption[] = models
    .filter((m) => m.kind === 'layout')
    .map((m) => ({ value: m.name, label: m.name, hint: m.version }));

  return (
    <SettingsRow label="区域模型" subtitle="models/ 下的模型目录名；界面识别不准时可训练自定义模型">
      <div className="flex gap-1.5">
        <Combobox value={value} options={options} onChange={onChange} className="w-52" />
        <IconButton
          variant="ghost"
          size="sm"
          data-tip="训练自定义模型"
          onClick={() => setTrainingOpen(true)}
        >
          <GraduationCap className="size-4" />
        </IconButton>
      </div>
      <TrainingDialog open={trainingOpen} onClose={() => setTrainingOpen(false)} />
    </SettingsRow>
  );
}

function ContactsGroup() {
  const [contacts, setContacts] = useState<import('../lib/types').ContactDto[]>([]);
  const [scenarios, setScenarios] = useState<ScenarioDto[]>([]);
  const [name, setName] = useState('');
  const [profile, setProfile] = useState('casual');
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    void api.listContacts().then(setContacts);
    void api.listScenarios().then(setScenarios);
  }, []);

  const setProfileOf = (n: string, p: string) => {
    setSaving(n);
    void api
      .setContactProfile(n, p)
      .then(() => api.listContacts())
      .then(setContacts)
      .finally(() => setSaving(null));
  };

  const addContact = () => {
    if (!name.trim()) return;
    setProfileOf(name.trim(), profile);
    setName('');
  };

  /** 画像下拉：未标记（删除条目，回落正式）+ 全部场景 */
  const profileOptions: ComboOption[] = [
    { value: 'none', label: '未标记', hint: '按正式处理' },
    ...scenarios.map((s) => ({ value: s.id, label: s.name })),
  ];

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
          <Combobox
            value={profile}
            options={scenarios.map((s) => ({ value: s.id, label: s.name }))}
            onChange={setProfile}
            className="w-32"
          />
          <button
            className="shrink-0 rounded-lg bg-[var(--brand)] px-4 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
            disabled={!name.trim()}
            onClick={addContact}
          >
            标记
          </button>
        </div>
      </SettingsRow>
      {contacts.map((c) => (
        <SettingsRow key={c.name} label={c.name}>
          <Combobox
            value={c.profile}
            options={profileOptions}
            disabled={saving === c.name}
            onChange={(v) => setProfileOf(c.name, v)}
            className="w-40"
          />
        </SettingsRow>
      ))}
    </SettingsGroup>
  );
}

/* ── 场景管理：场景列表 + 词库 ── */

/** 判定基线选项（L2 阈值与模型头按基线复用；对应高级设置的基线阈值配置） */
const BASE_OPTIONS: ComboOption[] = [
  { value: 'formal', label: '正式', hint: '较严' },
  { value: 'casual', label: '个人', hint: '较宽' },
];

const BASE_LABEL: Record<string, string> = { formal: '正式基线', casual: '个人基线' };

/** 场景管理页：上方场景列表（内置「正式」「个人」+ 自定义，共 ≤10），下方词库子分类 */
function ScenariosPage() {
  const [scenarios, setScenarios] = useState<ScenarioDto[]>([]);
  useEffect(() => {
    void api.listScenarios().then(setScenarios);
  }, []);
  return (
    <div className="mx-auto max-w-3xl">
      <h2 className="mb-1 text-xl font-semibold tracking-tight text-[var(--text-primary)]">场景管理</h2>
      <p className="mb-5 text-[13px] text-[var(--text-tertiary)]">
        场景决定对聊天对象的判定尺度；词库规则与对象画像均按场景生效。「正式」「个人」为内置场景。
      </p>
      <ScenarioList scenarios={scenarios} onChange={setScenarios} />
      <RulesEditor scenarios={scenarios} />
    </div>
  );
}

function ScenarioList({
  scenarios,
  onChange,
}: {
  scenarios: ScenarioDto[];
  onChange: (s: ScenarioDto[]) => void;
}) {
  const [dialog, setDialog] = useState<{ mode: 'add' } | { mode: 'edit'; target: ScenarioDto } | null>(null);
  const [confirming, setConfirming] = useState<ScenarioDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    void api
      .listScenarios()
      .then(onChange)
      .catch((e: Error) => setError(e.message || '场景加载失败'));

  const remove = (s: ScenarioDto) => {
    void api
      .removeScenario(s.id)
      .then(refresh)
      .catch((e: Error) => setError(e.message || '删除失败'))
      .finally(() => setConfirming(null));
  };

  return (
    <section className="mb-8">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">场景</h3>
        <span className="text-xs text-[var(--text-tertiary)]">{scenarios.length}/10</span>
        <button
          className="flex items-center gap-1.5 rounded-lg bg-[var(--brand)] px-3.5 py-2 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
          disabled={scenarios.length >= 10}
          onClick={() => setDialog({ mode: 'add' })}
        >
          <Plus className="size-4" />
          新增场景
        </button>
      </div>

      {error && (
        <div className="mb-3 rounded-lg bg-[var(--danger-soft)] px-4 py-2 text-sm text-[var(--danger)]">
          {error}
        </div>
      )}

      <div className="space-y-2">
        {scenarios.map((s) => (
          <div key={s.id} className="flex h-12 items-center gap-2.5 rounded-xl bg-[var(--group-bg)] px-4">
            <span className="text-sm font-medium text-[var(--text-primary)]">{s.name}</span>
            <span className="rounded-md bg-[var(--input-bg)] px-1.5 py-0.5 text-xs text-[var(--text-tertiary)]">
              {s.fixed ? '内置' : BASE_LABEL[s.base] ?? s.base}
            </span>
            {!s.fixed && (
              <div className="ml-auto flex gap-1">
                <IconButton variant="ghost" size="sm" data-tip="编辑" onClick={() => setDialog({ mode: 'edit', target: s })}>
                  <Pencil className="size-4" />
                </IconButton>
                <IconButton variant="ghost" size="sm" data-tip="删除" onClick={() => setConfirming(s)}>
                  <Trash2 className="size-4 text-[var(--danger)]" />
                </IconButton>
              </div>
            )}
            {s.fixed && <span className="ml-auto text-xs text-[var(--text-tertiary)]">不可修改</span>}
          </div>
        ))}
        {scenarios.length === 0 && (
          <p className="py-8 text-center text-sm text-[var(--text-tertiary)]">场景加载中…</p>
        )}
      </div>

      {dialog && (
        <ScenarioDialog
          initial={dialog.mode === 'edit' ? dialog.target : null}
          onClose={() => setDialog(null)}
          onSaved={refresh}
        />
      )}
      {confirming && (
        <Modal
          open
          title="删除场景"
          onClose={() => setConfirming(null)}
          footer={
            <>
              <ModalButton
                className="bg-[var(--danger)] text-white hover:opacity-90"
                onClick={() => remove(confirming)}
              >
                删除
              </ModalButton>
              <ModalButton onClick={() => setConfirming(null)}>取消</ModalButton>
            </>
          }
        >
          <p>
            删除「{confirming.name}」后，适用该场景的词库规则将不再生效，标记为该场景的聊天对象一律按「正式」处理。
          </p>
        </Modal>
      )}
    </section>
  );
}

/** 新增/编辑场景对话框 */
function ScenarioDialog({
  initial,
  onClose,
  onSaved,
}: {
  /** null = 新增 */
  initial: ScenarioDto | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? '');
  const [base, setBase] = useState<'formal' | 'casual'>(initial?.base ?? 'formal');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const save = () => {
    setBusy(true);
    setError(null);
    const p = initial ? api.updateScenario(initial.id, name, base) : api.addScenario(name, base);
    void p
      .then(() => {
        onSaved();
        onClose();
      })
      .catch((e: Error) => setError(e.message || '保存失败'))
      .finally(() => setBusy(false));
  };

  return (
    <Modal
      open
      title={initial ? '编辑场景' : '新增场景'}
      onClose={onClose}
      footer={
        <>
          <ModalButton variant="primary" disabled={busy || !name.trim()} onClick={save}>
            保存
          </ModalButton>
          <ModalButton disabled={busy} onClick={onClose}>
            取消
          </ModalButton>
        </>
      }
    >
      <label className="mb-1.5 block text-[13px] font-medium text-[var(--text-secondary)]">场景名</label>
      <input
        autoFocus
        className="h-9 w-full rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]"
        value={name}
        placeholder="如：工作群、家人"
        maxLength={12}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === 'Enter' && name.trim() && save()}
      />
      <div className="mt-4 flex items-center gap-3">
        <label className="shrink-0 text-[13px] font-medium text-[var(--text-secondary)]">判定基线</label>
        <Combobox value={base} options={BASE_OPTIONS} onChange={(v) => setBase(v as 'formal' | 'casual')} className="w-44" />
      </div>
      <p className="mt-3 text-xs leading-relaxed text-[var(--text-tertiary)]">
        基线决定语义模型的判定阈值（正式较严、个人较宽），词库规则始终按场景本身生效。
      </p>
      {error && <p className="mt-2.5 text-xs text-[var(--warning)]">{error}</p>}
    </Modal>
  );
}

const MATCH_OPTIONS: ComboOption[] = [
  { value: 'word', label: '整词' },
  { value: 'substring', label: '包含' },
  { value: 'regex', label: '正则' },
];

/** 词库编辑器（即时保存，移除显式保存按钮）。场景管理页的子分类。 */
function RulesEditor({ scenarios }: { scenarios: ScenarioDto[] }) {
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
      { pattern: '', match: 'substring', applies_to: ['all'] },
    ];
    setRules(updated);
  };

  const remove = (i: number) => {
    const updated = rules.filter((_, j) => j !== i);
    setRules(updated);
    save(updated);
  };

  /** 适用场景选项：全部 + 各场景（动态取自场景管理） */
  const appliesOptions: ComboOption[] = [
    { value: 'all', label: '全部场景' },
    ...scenarios.map((s) => ({ value: s.id, label: s.name })),
  ];

  const inputCls =
    'h-9 min-w-0 rounded-lg bg-[var(--input-bg)] px-3 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]';

  return (
    <section>
      <div className="mb-3 flex items-center justify-between">
        <div>
          <h3 className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">词库</h3>
          <p className="mt-0.5 text-xs text-[var(--text-tertiary)]">
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
            <Combobox
              value={r.match}
              options={MATCH_OPTIONS}
              onChange={(v) => update(i, { match: v })}
              className="w-24"
            />
            <Combobox
              value={r.applies_to[0] ?? 'all'}
              options={appliesOptions}
              onChange={(v) => update(i, { applies_to: [v] })}
              className="w-32"
            />
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
    </section>
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

/** 外部链接（主题色文字，点击经后端在外部浏览器打开；后端仅放行 https） */
function ExtLink({ url, children }: { url: string; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={() => void api.openExternal(url).catch(() => {})}
      className="font-medium text-[var(--brand)] underline underline-offset-2 transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
    >
      {children}
    </button>
  );
}

/** 微信官方协议（腾讯发布；出处见 docs/合规与风险说明.md §2.1/§2.2） */
const WECHAT_AGREEMENT =
  'https://weixin.qq.com/cgi-bin/readtemplate?lang=zh_CN&t=weixin_agreement&s=default';
const WECHAT_PERSONAL_RULES = 'https://weixin.qq.com/agreement/personal_account?lang=zh_CN';

function AboutPage() {
  return (
    <div className="mx-auto max-w-2xl">
      <AboutHero name={APP_NAME} version={APP_VERSION} />
      <SettingsSection>
        <SettingsGroup label="简介">
          <SettingsRow stacked>
            <p className="text-sm leading-relaxed text-[var(--text-secondary)]">
                {APP_NAME}是一款危险言语提前拦截工具：在消息发出前截取屏幕画面、在本机识别文字，
              发现可能引发风险的措辞时弹窗提醒，帮你避免一时冲动发出不当言论。
              全程纯本地运行，不依赖任何网络服务。
            </p>
          </SettingsRow>
        </SettingsGroup>

        <SettingsGroup label="风险提示">
          {/* 重点条款：主题色左边线 + 主题色强调，阅读时不可错过 */}
          <SettingsRow stacked>
            <div className="rounded-xl border-l-[3px] border-[var(--brand)] bg-[var(--brand-soft)] px-4 py-3 text-sm leading-relaxed text-[var(--text-secondary)]">
              <p>
                本软件通过
                <span className="font-semibold text-[var(--brand)]">截取屏幕画面并识别文字</span>
                的方式工作，可能涉及第三方平台关于自动化操作与屏幕内容采集的相关条款，
                由此产生的账号风险由使用者自行承担。使用前请阅读
                <ExtLink url={WECHAT_AGREEMENT}>《微信软件许可及服务协议》</ExtLink>
                与
                <ExtLink url={WECHAT_PERSONAL_RULES}>《微信个人账号使用规范》</ExtLink>
                并自行评估。
              </p>
            </div>
          </SettingsRow>
          <SettingsRow label="原则和底线" stacked>
            <ul className="space-y-1 text-sm leading-relaxed text-[var(--text-secondary)]">
              <li>· 不注入或修改微信程序</li>
              <li>· 不读取或解密微信聊天记录文件</li>
              <li>· 不替你发送任何消息</li>
              <li>· 不把聊天内容上传到任何服务器（纯本地方案）</li>
              <li>· 绝不触碰法律法规底线：不开发、不内置任何绕过监管或对抗审查的功能</li>
              <li>· 绝不采集与拦截无关的数据：识别仅在内存中进行，落盘内容不包含消息原文</li>
            </ul>
          </SettingsRow>
        </SettingsGroup>

        <SettingsGroup label="开源信息">
          <SettingsRow label="源码地址">
            <ExtLink url={REPO_URL}>{REPO_URL.replace('https://', '')}</ExtLink>
          </SettingsRow>
          <SettingsRow label="作者">
            <ExtLink url={AUTHOR_URL}>{AUTHOR}</ExtLink>
          </SettingsRow>
          <SettingsRow label="许可证">
            <ExtLink url={`${REPO_URL}/blob/main/LICENSE`}>{LICENSE} License</ExtLink>
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
    </div>
  );
}
