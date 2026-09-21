import { useEffect, useMemo, useState, useRef } from 'react';
import { Shield, ShieldAlert, ShieldOff, Plus, Trash2, Pencil, FolderOpen, FolderCog, Loader2, Moon, Sun, Monitor, GraduationCap, SlidersHorizontal, MessageSquareWarning, Layers, Info, Palette, MonitorCog, ShieldCheck, ScanText, Users, BookText, Cpu, ScrollText, FileText, AlertTriangle, Code2, Eye, type LucideIcon } from 'lucide-react';
import { cn } from '../lib/utils';
import type { ConfigFieldDto, ContactDto, ModelDto, RuleDto, ScenarioDto, StatusPayload } from '../lib/types';
import * as api from '../lib/commands';
import { onStats, onStatus, onModels } from '../lib/events';
import { SchemaField } from './SchemaField';
import { Switch } from '../components/Switch';
import { Checkbox } from '../components/Checkbox';
import { Modal, ModalButton } from '../components/Modal';
import { IconButton } from '../components/IconButton';
import { Combobox, type ComboOption } from '../components/Combobox';
import { ExtLink } from '../components/ExtLink';
import { SettingsGroup, SettingsRow, SettingsSection } from '../components/SettingsGroup';
import { AboutHero } from './AboutHero';
import { useTintedLogo } from '../lib/logoTint';
import { AlertCard, AlertPreviewFrame, PREVIEW_ALERT } from '../alert/AlertCard';
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
  DEFAULT_THEME,
  DEFAULT_ACCENT_ID,
  type Theme,
} from '../lib/theme';

/** 分类导航（FR-UI-03）：按用户视角归组——日常防护开关 + 场景与词库 +
 *  面向开发者的技术参数（高级设置）。 */
const CATEGORY_ORDER = ['通用', '消息防护', '场景管理', '高级设置', '关于'] as const;
type Category = (typeof CATEGORY_ORDER)[number];

/** 分类导航图标（左侧列表每项前置）。 */
const CATEGORY_ICON: Record<Category, LucideIcon> = {
  通用: SlidersHorizontal,
  消息防护: MessageSquareWarning,
  场景管理: Layers,
  高级设置: Cpu,
  关于: Info,
};

/** 分组标题图标（右侧各分组卡片标题前置；键为分组 label）。 */
const GROUP_ICON: Record<string, LucideIcon> = {
  外观: Palette,
  系统: MonitorCog,
  目标程序: Shield,
  拦截行为: ShieldCheck,
  语义判定: MessageSquareWarning,
  聊天对象画像: Users,
  拦截引擎: Cpu,
  区域识别: ScanText,
  文字识别: ScanText,
  诊断: ScrollText,
  日志: ScrollText,
  场景: Layers,
  词库: BookText,
  简介: FileText,
  风险提示: AlertTriangle,
  开源信息: Code2,
};

/** 页面大标题（图标 + 文字）：右侧各分类顶部标题统一样式。 */
function PageTitle({
  icon: Icon,
  className,
  children,
}: {
  icon: LucideIcon;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <h2
      className={cn(
        'flex items-center gap-2.5 text-xl font-semibold tracking-tight text-[var(--text-primary)]',
        className,
      )}
    >
      <Icon className="size-5 shrink-0 text-[var(--brand)]" strokeWidth={2.2} />
      {children}
    </h2>
  );
}

/** 技术参数键（判定有效期/防抖等一般人无需关心的）→ 高级设置 */
const ADVANCED_KEYS = new Set([
  'guard.verdict_ttl_ms',
  'guard.foreground_debounce_ms',
]);

/** 内置目标程序（下拉选择）。
 *  展示名 = 中文名（进程名），与任务管理器一致；value 必须与后端
 *  `target.process_name` 匹配的可执行文件名。 */
const TARGET_APPS: ComboOption[] = [
  { value: 'Weixin.exe', label: '微信（Weixin.exe）' },
  { value: 'notepad.exe', label: '记事本（notepad.exe）', hint: '验证用' },
];

/** 下拉哨兵值：仅表示「进入自定义输入态」，永远不会写进配置 */
const CUSTOM_APP_VALUE = '__custom__';

/** 消息防护页的分组顺序与标题 */
const WECHAT_GROUPS: { title: string; keys: string[] }[] = [
  { title: '目标程序', keys: ['target.process_name'] },
  {
    title: '拦截行为',
    keys: ['guard.enabled', 'guard.send_key', 'alert.timeout_secs', 'alert.shake'],
  },
];

/** 高级设置的分组顺序与标题 */
const ADVANCED_GROUPS: { title: string; keys: string[] }[] = [
  { title: '拦截引擎', keys: [...ADVANCED_KEYS] },
  {
    title: '语义判定',
    keys: ['sem.model', 'sem.l2_enabled', 'sem.threshold.formal', 'sem.threshold.casual'],
  },
  { title: '区域识别', keys: ['layout.model', 'layout.conf_threshold', 'layout.nms_iou'] },
  { title: '文字识别', keys: ['ocr.upscale', 'ocr.min_conf', 'ocr.noise_words'] },
  { title: '诊断', keys: ['debug.save_images', 'debug.image_dirs_limit'] },
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

  /** logo 动态变色：logoSrc 已是主题色着色图；状态只叠**明暗**滤镜（不降饱和）。
   *  守护中 = 主题色本色；等待目标 = 略暗；挂起/冷却 = 压暗；暂停(禁用) = 最暗。
   *  为何压暗而非降饱和：去饱和后接近白/灰，与浅色背景难以区分（用户反馈）。 */
  const [accentHue, setAccentHue] = useState(() => getAccent().hue);
  const [accentSat, setAccentSat] = useState(() => getAccent().sat);
  useEffect(() => {
    const on = (e: Event) => {
      const a = (e as CustomEvent<{ hue: number; sat: number }>).detail;
      setAccentHue(a.hue);
      setAccentSat(a.sat);
    };
    window.addEventListener('main:accent', on);
    return () => window.removeEventListener('main:accent', on);
  }, []);
  const logoSrc = useTintedLogo(accentHue, accentSat);
  const logoFilter =
    status == null
      ? 'brightness(0.6)'
      : status.state === 'active'
        ? status.target_found
          ? undefined
          : 'brightness(0.8)'
        : status.state === 'paused'
          ? 'brightness(0.45)'
          : 'brightness(0.6)';

  /** 渲染一组 schema 字段（目标程序/区域模型特殊渲染为自定义行） */
  const renderField = (key: string) => {
    if (key === 'target.process_name') {
      const def = byKey.get(key)?.default;
      return (
        <TargetAppRow
          key={key}
          value={values[key] ?? def}
          defaultValue={def}
          onChange={(v) => change(key, v)}
        />
      );
    }
    if (key === 'layout.model') {
      const def = String(byKey.get(key)?.default ?? 'dc-layout-wechat');
      return (
        <LayoutModelRow
          key={key}
          value={String(values[key] ?? def)}
          defaultValue={def}
          onChange={(v) => change(key, v)}
        />
      );
    }
    if (key === 'sem.model') {
      const def = String(byKey.get(key)?.default ?? 'bge-large');
      return (
        <SemModelRow
          key={key}
          value={String(values[key] ?? def)}
          defaultValue={def}
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
            src={logoSrc}
            alt={APP_NAME}
            className="size-9 rounded-xl object-cover shadow-[var(--shadow-sm)] transition-[filter] duration-300"
            style={{ filter: logoFilter }}
          />
          <div className="flex flex-1 flex-col leading-tight">
            <span className="text-item font-semibold tracking-tight text-[var(--text-primary)]">{APP_NAME}</span>
            <div className="mt-0.5 flex items-center gap-1.5 text-xs">
              <StateIcon className={cn('size-3.5', stateColor)} strokeWidth={2} />
              <span className="text-[var(--text-tertiary)]">{stateText}</span>
            </div>
          </div>
        </div>

        <div className="space-y-1.5">
          {CATEGORY_ORDER.map((c) => {
            const Icon = CATEGORY_ICON[c];
            return (
              <button
                key={c}
                onClick={() => setActive(c)}
                className={cn(
                  'flex w-full items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-sm font-semibold transition-all duration-150',
                  active === c
                    ? 'bg-[var(--brand)] text-[var(--brand-text)] shadow-[var(--shadow-sm)]'
                    : 'text-[var(--text-secondary)] hover:bg-[var(--hover-overlay)] hover:text-[var(--text-primary)]',
                )}
              >
                <Icon className="size-4 shrink-0" strokeWidth={2.2} />
                {c}
              </button>
            );
          })}
        </div>

        {stats && (
          <div className="mt-auto mb-2 mx-1 rounded-xl bg-[var(--group-bg)] px-5 py-4">
            <div className="flex items-baseline justify-center gap-1.5 text-sm">
              <span className="text-[var(--text-tertiary)]">今日拦截</span>
              <strong className="font-semibold tabular-nums tracking-tight text-[var(--brand)]">
                {stats.today_blocked}
              </strong>
              <span className="text-[var(--text-tertiary)]">次</span>
            </div>
          </div>
        )}
      </nav>

      {/* 右侧设置区：提亮一档（Win11：左导航灰、右内容白） */}
      <div className="min-w-0 flex-1 overflow-y-auto rounded-tl-xl bg-[var(--panel-bg)] px-8 py-6">
        {active === '通用' && <GeneralPage />}
        {active === '消息防护' && (
          <div className="mx-auto max-w-2xl">
            <PageTitle icon={CATEGORY_ICON['消息防护']} className="mb-5">消息防护</PageTitle>
            <SettingsSection>
              {WECHAT_GROUPS.map((g) => (
                <SettingsGroup key={g.title} label={g.title} icon={GROUP_ICON[g.title]}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
              <AlertPreviewGroup />
            </SettingsSection>
          </div>
        )}
        {active === '场景管理' && <ScenariosPage />}
        {active === '高级设置' && (
          <div className="mx-auto max-w-2xl">
            <PageTitle icon={CATEGORY_ICON['高级设置']} className="mb-1">高级设置</PageTitle>
            <p className="mb-5 text-label text-[var(--text-tertiary)]">
              面向开发与调试的技术参数，日常使用无需调整。
            </p>
            <SettingsSection>
              {ADVANCED_GROUPS.map((g) => (
                <SettingsGroup key={g.title} label={g.title} icon={GROUP_ICON[g.title]}>
                  {g.keys.map(renderField)}
                </SettingsGroup>
              ))}
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
      <PageTitle icon={CATEGORY_ICON['通用']} className="mb-5">通用</PageTitle>
      <SettingsSection>
        <AppearanceGroup />
        <SettingsGroup label="系统" icon={GROUP_ICON['系统']}>
          <AutostartRow />
          <DataDirRow />
        </SettingsGroup>
        <LogsGroup />
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
    { value: 'system', label: '系统', icon: Monitor },
  ];

  const applyAccentChoice = (a: (typeof ACCENTS)[number]) => {
    setAccent(a);
    setAccentId(a.id);
    // 托盘图标同步主题色（色相 + 饱和度；浏览器 dev 环境无 Tauri，静默失败）
    void api.setTrayAccent(a.hue, a.sat).catch(() => {});
  };

  return (
    <SettingsGroup label="外观" icon={GROUP_ICON['外观']}>
      <SettingsRow
        label="主题"
        subtitle="界面配色，跟随系统时随系统设置自动切换"
        dirty={theme !== DEFAULT_THEME}
        onReset={() => {
          setTheme(DEFAULT_THEME);
          setThemeState(DEFAULT_THEME);
        }}
      >
        <div className="flex gap-1 rounded-lg bg-[var(--input-bg)] p-1">
          {themeOptions.map(({ value, label, icon: Icon }) => (
            <button
              key={value}
              onClick={() => {
                setTheme(value);
                setThemeState(value);
              }}
              className={cn(
                'flex h-7 items-center gap-1.5 rounded-md px-3 text-label font-medium transition-all',
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
      <SettingsRow
        label="主题色"
        subtitle="强调色；托盘与界面图标同步变色"
        dirty={accentId !== DEFAULT_ACCENT_ID}
        onReset={() => {
          const def = ACCENTS.find((a) => a.id === DEFAULT_ACCENT_ID) ?? ACCENTS[0];
          applyAccentChoice(def);
        }}
      >
        <div className="flex items-center gap-2">
          {ACCENTS.map((a) => (
            <button
              key={a.id}
              onClick={() => applyAccentChoice(a)}
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
      <SettingsRow
        label="自定义鼠标指针"
        subtitle="品牌色箭头/圆环光标，关闭后使用系统指针"
        dirty={!cursorFx}
        onReset={() => toggleCursorFx(true)}
      >
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

  const setEnabled = (next: boolean) => {
    void import('@tauri-apps/plugin-autostart').then(async (m) => {
      if (next) await m.enable();
      else await m.disable();
      setOn(next);
    });
  };

  // 默认关闭：开启后显示 * 与还原图标
  return (
    <SettingsRow
      label="开机启动"
      subtitle="系统登录时自动运行"
      dirty={on}
      onReset={() => setEnabled(false)}
    >
      <Switch checked={on} onChange={setEnabled} label="开机启动" />
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
  /** 还原为默认目录确认框 */
  const [resetConfirm, setResetConfirm] = useState(false);
  const [resetting, setResetting] = useState(false);

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
      // 目标与当前目录相同：后端原样返回（files=0），不弹回执，静默结束
      if (r.files === 0 && r.bytes === 0 && r.target === r.previous) return;
      setInfo({ current: r.target, custom: true });
      setReport(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : '迁移失败');
    } finally {
      setBusy(false);
    }
  };

  /** 还原为默认目录：走与「设为默认」一致的流程（set_data_dir("") → 写指针 → 重启生效）。 */
  const resetToDefault = async () => {
    setResetting(true);
    setError(null);
    try {
      await api.setDataDir('');
      await api.restartApp();
    } catch (e) {
      setError(e instanceof Error ? e.message : '还原失败');
      setResetting(false);
      setResetConfirm(false);
    }
  };

  return (
    <SettingsRow
      label="数据目录"
      subtitle={info ? (info.custom ? `${info.current}（自定义）` : info.current) : '读取中…'}
      dirty={info?.custom ?? false}
      onReset={() => setResetConfirm(true)}
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
      {resetConfirm && (
        <Modal
          open
          title="还原为默认数据目录"
          onClose={() => !resetting && setResetConfirm(false)}
          footer={
            <>
              <ModalButton variant="primary" disabled={resetting} onClick={() => void resetToDefault()}>
                {resetting ? '还原中…' : '还原并重启'}
              </ModalButton>
              <ModalButton disabled={resetting} onClick={() => setResetConfirm(false)}>
                取消
              </ModalButton>
            </>
          }
        >
          <p>
            将数据目录指针改回系统默认位置，重启后生效。当前自定义目录下的数据不会被移动或删除，
            如需保留请自行迁移。
          </p>
        </Modal>
      )}
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
          label={`删除旧目录：${report.previous}`}
        />
        <p className="mt-1.5 pl-6 text-xs text-[var(--text-tertiary)]">
          旧目录下所有内容都会删除，包括系统默认目录里迁移前留下的数据；指向新目录的指针会保留。
        </p>
      </div>
      {cleanupError && (
        <p className="mt-2.5 text-xs text-[var(--warning)]">旧目录清理失败：{cleanupError}</p>
      )}
    </Modal>
  );
}

/* ── 消息防护：目标程序下拉 + 画像 ── */

/** 目标程序行：内置应用下拉 + 自定义进程名输入。
 *
 *  `target.process_name` 是后端热更新的唯一数据源：下拉选中即写该键；
 *  自定义进程名走输入框写回同一键，不另开「自定义键」——否则「当前生效的是哪个」会有两个答案。
 */
function TargetAppRow({
  value,
  defaultValue,
  onChange,
}: {
  value: unknown;
  defaultValue?: unknown;
  onChange: (v: unknown) => void;
}) {
  const current = String(value ?? '');
  const isKnown = TARGET_APPS.some((o) => o.value.toLowerCase() === current.toLowerCase());
  // 当前值不在内置库内 → 默认展开输入态；用户主动选「自定义…」也进入输入态
  const [customMode, setCustomMode] = useState(!isKnown);
  const [custom, setCustom] = useState(current);

  // 外部值变化时（如初次加载完成、热更新回流）同步草稿，避免残留旧值
  useEffect(() => {
    setCustom(current);
    setCustomMode(!TARGET_APPS.some((o) => o.value.toLowerCase() === current.toLowerCase()));
  }, [current]);

  const applyCustom = () => {
    const v = custom.trim();
    if (v && v !== current) onChange(v);
  };

  const def = String(defaultValue ?? '');
  const dirty = def !== '' && current.toLowerCase() !== def.toLowerCase();

  return (
    <SettingsRow
      label="防护应用"
      subtitle="仅拦截该窗口的发送按键"
      dirty={dirty}
      onReset={() => onChange(def)}
    >
      <div className="flex flex-col items-end gap-2">
        <Combobox
          value={customMode ? CUSTOM_APP_VALUE : current}
          options={[
            ...TARGET_APPS,
            { value: CUSTOM_APP_VALUE, label: '自定义…', hint: '手动填写进程名' },
          ]}
          onChange={(v) => {
            if (v === CUSTOM_APP_VALUE) {
              setCustomMode(true);
              setCustom(current);
            } else {
              setCustomMode(false);
              onChange(v);
            }
          }}
          fitContent
        />
        {customMode && (
          <input
            autoFocus
            className="h-9 w-40 rounded-lg bg-[var(--input-bg)] px-3.5 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 placeholder:text-[var(--text-tertiary)] hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]"
            value={custom}
            placeholder="如 MsgTest.exe（含扩展名）"
            maxLength={128}
            onChange={(e) => setCustom(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && applyCustom()}
            onBlur={applyCustom}
          />
        )}
      </div>
    </SettingsRow>
  );
}

/** 区域模型行：下拉列出 models/ 有效 layout 模型（目录监听自动刷新）+ 自助训练入口 */
function LayoutModelRow({
  value,
  defaultValue,
  onChange,
}: {
  value: string;
  defaultValue?: string;
  onChange: (v: string) => void;
}) {
  const [models, setModels] = useState<ModelDto[]>([]);
  const [trainingOpen, setTrainingOpen] = useState(false);

  useEffect(() => {
    void api.listModels().then(setModels);
    const un = onModels(setModels);
    return () => void un.then((f) => f());
  }, []);

  const options: ComboOption[] = models
    .filter((m) => m.kind === 'layout')
    .map((m) => ({
      value: m.name,
      label: m.name,
      hint: m.source === 'builtin' ? '内置' : '用户',
    }));

  const dirty = defaultValue != null && value !== defaultValue;

  return (
    <SettingsRow
      label="区域模型"
      subtitle="models/ 下的模型目录名；界面识别不准时可训练自定义模型"
      dirty={dirty}
      onReset={() => defaultValue != null && onChange(defaultValue)}
    >
      <div className="flex gap-1.5">
        <Combobox value={value} options={options} onChange={onChange} className="w-40" />
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

/** 语义模型行：下拉列出 models/ 有效 sem 模型（内置 bge-large + 用户放入的替换模型）。
 *  切换后下次启动生效（模型在 init 读取，与区域模型一致）。 */
function SemModelRow({
  value,
  defaultValue,
  onChange,
}: {
  value: string;
  defaultValue?: string;
  onChange: (v: string) => void;
}) {
  const [models, setModels] = useState<ModelDto[]>([]);

  useEffect(() => {
    void api.listModels().then(setModels);
    const un = onModels(setModels);
    return () => void un.then((f) => f());
  }, []);

  const options: ComboOption[] = models
    .filter((m) => m.kind === 'sem')
    .map((m) => ({
      value: m.name,
      label: m.name,
      hint: m.source === 'builtin' ? '内置' : '用户',
    }));

  const dirty = defaultValue != null && value !== defaultValue;

  return (
    <SettingsRow
      label="语义模型"
      subtitle="models/ 下 kind=sem 的模型；默认内置 bge-large，放入自训练/替换模型后可切换（重启生效）"
      dirty={dirty}
      onReset={() => defaultValue != null && onChange(defaultValue)}
    >
      <Combobox value={value} options={options} onChange={onChange} className="w-40" />
    </SettingsRow>
  );
}

/** 聊天对象画像：内联可编辑行（名称输入框 + 画像下拉 + 删除），与词库条目同构。
 *
 *  后端只有 `set_contact_profile(name, profile)` 一个写入口（profile="none" 即删除），
 *  「改名」因此是一次「新名写入 + 旧名删除」的迁移——故名称失焦时整体提交，
 *  而非逐字符写盘。
 */
function ContactsGroup() {
  const [contacts, setContacts] = useState<ContactDto[]>([]);
  const [scenarios, setScenarios] = useState<ScenarioDto[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<ContactDto | null>(null);
  /** 新增草稿：仅存在于前端，name 非空提交后才落库（后端拒绝空名）。null=无草稿。 */
  const [draft, setDraft] = useState<{ name: string; profile: string } | null>(null);
  const draftRowRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void api.listContacts().then(setContacts);
    void api.listScenarios().then(setScenarios);
  }, []);

  const defaultProfile = () => scenarios[0]?.id ?? '';

  /** 统一写路径：失败落到 error 并回读真实状态（杜绝乐观残影）。 */
  const commit = (key: string, write: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    return write()
      .then(() => api.listContacts())
      .then(setContacts)
      .catch((e: Error) => {
        setError(e.message || '保存失败');
        return api.listContacts().then(setContacts);
      })
      .finally(() => setBusy(null));
  };

  /** 点「添加对象」：插入一行空草稿（下方 autoFocus 到名称输入框）。 */
  const startDraft = () => {
    if (draft) return; // 已有未完成草稿，避免堆叠空行
    setDraft({ name: '', profile: defaultProfile() });
  };

  /** 提交草稿：名称去空为空则丢弃，否则写库并清空草稿。 */
  const commitDraft = () => {
    if (!draft) return;
    const n = draft.name.trim();
    if (!n || !draft.profile) {
      setDraft(null);
      return;
    }
    const p = draft.profile;
    setDraft(null);
    void commit(n, () => api.setContactProfile(n, p));
  };

  const removeContact = (c: ContactDto) => {
    void commit(c.name, () => api.setContactProfile(c.name, 'none')).then(() => setConfirming(null));
  };

  /** 改名 = 写入新名 + 删除旧名；名称未变时只更新画像。空名视为无效，还原不删除（删除走垃圾桶+确认）。 */
  const renameContact = (from: string, to: string, p: string) => {
    if (!to || to === from) return;
    void commit(to, async () => {
      await api.setContactProfile(to, p);
      await api.setContactProfile(from, 'none');
    });
  };

  /** 画像下拉：只有各场景。删除是独立动作（右侧垃圾桶 → 确认框），不在这里表达。 */
  const profileOptions: ComboOption[] = scenarios.map((s) => ({ value: s.id, label: s.name }));

  /** 引用已删除场景的残留画像：补一个回落项，让界面显示可读文案而不是内部标识符。 */
  const missingProfiles: ComboOption[] = [
    ...new Set(contacts.map((c) => c.profile).filter((p) => !scenarios.some((s) => s.id === p))),
  ].map((p) => ({ value: p, label: `${p}（已删除，按正式处理）` }));

  const inputCls =
    'h-9 min-w-0 rounded-lg bg-[var(--input-bg)] px-3 text-sm text-[var(--text-primary)] outline-none transition-all duration-150 hover:bg-[var(--active-overlay)] focus:bg-[var(--panel-bg)] focus:shadow-[inset_0_0_0_1.5px_var(--brand)]';

  return (
    <section className="mb-8">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <h3 className="flex items-center gap-2 text-item font-semibold tracking-tight text-[var(--text-primary)]">
            <Users className="size-4 shrink-0 text-[var(--brand)]" strokeWidth={2.2} />
            聊天对象画像
          </h3>
          <p className="mt-0.5 text-xs text-[var(--text-tertiary)]">
            未标记的对象一律按「正式」保守处理；画像由 OCR 识别到的对象名匹配
          </p>
        </div>
        <button
          className="flex shrink-0 items-center gap-1.5 rounded-lg bg-[var(--brand)] px-3.5 py-2 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
          disabled={scenarios.length === 0 || draft !== null}
          onClick={startDraft}
        >
          <Plus className="size-4" />
          添加对象
        </button>
      </div>

      {error && (
        <div className="mb-3 rounded-lg bg-[var(--danger-soft)] px-4 py-2 text-sm text-[var(--danger)]">
          {error}
        </div>
      )}

      <div className="space-y-2">
        {contacts.map((c) => (
          <div
            key={c.name}
            className="flex items-center gap-2 rounded-xl bg-[var(--group-bg)] px-3 py-2"
          >
            <input
              className={cn(inputCls, 'flex-1')}
              defaultValue={c.name}
              maxLength={64}
              disabled={busy !== null}
              onKeyDown={(e) => {
                if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                if (e.key === 'Escape') {
                  (e.target as HTMLInputElement).value = c.name;
                  (e.target as HTMLInputElement).blur();
                }
              }}
              onBlur={(e) => {
                const to = e.target.value.trim();
                // 已有条目名不能清空（清空视为无效）→ 还原原名，不误删既有数据
                if (!to) {
                  e.target.value = c.name;
                  return;
                }
                renameContact(c.name, to, c.profile);
              }}
            />
            <Combobox
              value={c.profile}
              options={[...profileOptions, ...missingProfiles]}
              disabled={busy !== null}
              onChange={(v) => commit(c.name, () => api.setContactProfile(c.name, v))}
              className="w-32"
            />
            <IconButton
              variant="ghost"
              data-tip="删除"
              disabled={busy !== null}
              onClick={() => setConfirming(c)}
            >
              <Trash2 className="size-4 text-[var(--danger)]" />
            </IconButton>
          </div>
        ))}

        {/* 新增草稿行：autoFocus 名称；名称空且焦点离开本行 → 自动删除（焦点在下拉框/删除内不删）。 */}
        {draft && (
          <div
            ref={draftRowRef}
            className="flex items-center gap-2 rounded-xl bg-[var(--group-bg)] px-3 py-2"
          >
            <input
              autoFocus
              className={cn(inputCls, 'flex-1')}
              value={draft.name}
              placeholder="对象名（与聊天窗口显示名一致）"
              maxLength={64}
              onChange={(e) => setDraft((d) => (d ? { ...d, name: e.target.value } : d))}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commitDraft();
                if (e.key === 'Escape') setDraft(null);
              }}
              onBlur={(e) => {
                // 焦点仍在本行内（移到下拉框/删除按钮）→ 不处理，等真正离开
                if (draftRowRef.current?.contains(e.relatedTarget as Node | null)) return;
                commitDraft(); // 空名在 commitDraft 内被丢弃，非空则落库
              }}
            />
            <Combobox
              value={draft.profile}
              options={profileOptions}
              onChange={(v) => {
                const name = draft.name.trim();
                // 名称已填 → 选完画像即落库；否则只更新草稿，等用户补名称
                if (name) {
                  setDraft(null);
                  void commit(name, () => api.setContactProfile(name, v));
                } else {
                  setDraft((d) => (d ? { ...d, profile: v } : d));
                }
              }}
              className="w-32"
            />
            <IconButton variant="ghost" data-tip="取消" onClick={() => setDraft(null)}>
              <Trash2 className="size-4 text-[var(--danger)]" />
            </IconButton>
          </div>
        )}

        {contacts.length === 0 && !draft && (
          <p className="py-8 text-center text-sm text-[var(--text-tertiary)]">
            暂无对象，全部按「正式」基线处理
          </p>
        )}
      </div>

      {confirming && (
        <Modal
          open
          title="删除对象"
          onClose={() => setConfirming(null)}
          footer={
            <>
              <ModalButton
                className="bg-[var(--danger)] text-white hover:opacity-90"
                onClick={() => removeContact(confirming)}
              >
                删除
              </ModalButton>
              <ModalButton onClick={() => setConfirming(null)}>取消</ModalButton>
            </>
          }
        >
          <p>删除「{confirming.name}」后，该对象不再有专属画像，一律按「正式」基线处理。</p>
        </Modal>
      )}
    </section>
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
      <PageTitle icon={CATEGORY_ICON['场景管理']} className="mb-1">场景管理</PageTitle>
      <p className="mb-5 text-label text-[var(--text-tertiary)]">
        场景决定对聊天对象的判定尺度；词库规则与对象画像均按场景生效。「正式」「个人」为内置场景。
      </p>
      <ScenarioList scenarios={scenarios} onChange={setScenarios} />
      <ContactsGroup />
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
        <h3 className="flex items-center gap-2 text-item font-semibold tracking-tight text-[var(--text-primary)]">
          <Layers className="size-4 shrink-0 text-[var(--brand)]" strokeWidth={2.2} />
          场景
        </h3>
        <button
          className="flex items-center gap-1.5 rounded-lg bg-[var(--brand)] px-3.5 py-2 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
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
      <label className="mb-1.5 block text-label font-medium text-[var(--text-secondary)]">场景名</label>
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
        <label className="shrink-0 text-label font-medium text-[var(--text-secondary)]">判定基线</label>
        <Combobox value={base} options={BASE_OPTIONS} onChange={(v) => setBase(v as 'formal' | 'casual')} className="w-40" />
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
  const [rules, setRules] = useState<RuleDto[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const saveTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    void api.getRules().then(setRules);
  }, []);

  const save = (updatedRules: RuleDto[]) => {
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

  const update = (i: number, patch: Partial<RuleDto>) => {
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
          <h3 className="flex items-center gap-2 text-item font-semibold tracking-tight text-[var(--text-primary)]">
            <BookText className="size-4 shrink-0 text-[var(--brand)]" strokeWidth={2.2} />
            词库
          </h3>
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
          <div
            key={i}
            className="flex items-center gap-2 rounded-xl bg-[var(--group-bg)] px-3 py-2"
            // 词库对齐对象画像：条目失焦（焦点真正离开本行——不在输入框/下拉框/删除内）
            // 且 pattern 去空为空 → 自动删除空条目。下拉面板经 portal 渲染到 body，
            // relatedTarget 会落在行外，但选值走 onChange 即时更新且 pattern 非空时不触发删除，
            // 故仅对「留空未填」的行生效，不误删正在选下拉的非空行。
            onBlur={(e) => {
              if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
              if (r.pattern.trim() === '') remove(i);
            }}
          >
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
              className="w-32"
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

/* ── 消息防护：拦截效果预览 ── */

/** 拦截效果预览：用示例数据渲染一份真实弹窗（AlertCard 同源），纯展示不触发任何动作。
 *  让用户在设置里就能看到「拦截时会弹出什么」。 */
function AlertPreviewGroup() {
  const [contextOpen, setContextOpen] = useState(false);
  return (
    <SettingsGroup label="拦截效果预览" icon={Eye}>
      <div className="px-5 pb-4 pt-1">
        <p className="mb-3 text-label text-[var(--text-tertiary)]">
          下面是检测到危险内容时弹出的提示样式（示例，不影响实际设置）。
        </p>
        <AlertPreviewFrame>
          <AlertCard
            payload={PREVIEW_ALERT}
            contextOpen={contextOpen}
            onToggleContext={() => setContextOpen((v) => !v)}
          />
        </AlertPreviewFrame>
      </div>
    </SettingsGroup>
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
    <SettingsGroup label="日志" icon={GROUP_ICON['日志']}>
      <SettingsRow label="清空全部日志" subtitle="删除日志目录全部内容（不可恢复）">
        {confirming ? (
          <div className="flex gap-2">
            <button
              className="inline-flex h-9 items-center justify-center rounded-lg bg-[var(--danger)] px-3.5 text-label font-medium leading-none text-white shadow-[var(--shadow-sm)] transition-opacity hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
              disabled={clearing}
              onClick={clearLogs}
            >
              {/* YaHei 字形在行盒内偏低（内部 ascent/descent 不对称），flex 居中后仍偏下，
                  上移 1px 抵消，视觉才真正居中 */}
              <span className="-translate-y-px">{clearing ? '清空中...' : '确认清空'}</span>
            </button>
            <button
              className="inline-flex h-9 items-center justify-center rounded-lg bg-[var(--input-bg)] px-3.5 text-label leading-none text-[var(--text-secondary)] transition-colors hover:bg-[var(--active-overlay)] hover:text-[var(--text-primary)]"
              onClick={() => setConfirming(false)}
            >
              <span className="-translate-y-px">取消</span>
            </button>
          </div>
        ) : (
          <button
            className="inline-flex h-9 items-center justify-center rounded-lg bg-[var(--input-bg)] px-3.5 text-label font-medium leading-none text-[var(--text-primary)] transition-colors hover:bg-[var(--danger-soft)] hover:text-[var(--danger)]"
            onClick={() => setConfirming(true)}
          >
            <span className="-translate-y-px">清空全部日志</span>
          </button>
        )}
      </SettingsRow>
    </SettingsGroup>
  );
}

/* ── 关于 ── */

/** 微信官方协议（腾讯发布；出处见 docs/合规与风险说明.md §2.1/§2.2） */
export const WECHAT_AGREEMENT =
  'https://weixin.qq.com/cgi-bin/readtemplate?lang=zh_CN&t=weixin_agreement&s=default';
export const WECHAT_PERSONAL_RULES = 'https://weixin.qq.com/agreement/personal_account?lang=zh_CN';

function AboutPage() {
  return (
    <div className="mx-auto max-w-2xl">
      <AboutHero name={APP_NAME} version={APP_VERSION} />
      <SettingsSection>
        <SettingsGroup label="简介" icon={GROUP_ICON['简介']}>
          <SettingsRow stacked>
            <p className="text-sm leading-relaxed text-[var(--text-secondary)]">
                {APP_NAME}是一款危险言语提前拦截工具：在消息发出前智能分析聊天内容，
              识别是否误发、错发并弹窗提醒，尽可能帮你避免严重问题。
            </p>
          </SettingsRow>
        </SettingsGroup>

        <SettingsGroup label="风险提示" icon={GROUP_ICON['风险提示']}>
          {/* 重点条款：主题色强调，阅读时不可错过 */}
          <SettingsRow stacked>
            <div className="rounded-xl bg-[var(--brand-soft)] px-4 py-3 text-sm leading-relaxed text-[var(--text-secondary)]">
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
            </ul>
          </SettingsRow>
        </SettingsGroup>

        <SettingsGroup label="开源信息" icon={GROUP_ICON['开源信息']}>
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
