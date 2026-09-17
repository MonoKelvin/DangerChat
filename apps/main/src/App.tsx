import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SettingsRoot } from './settings/SettingsRoot';
import { NoticePage } from './notice/NoticePage';
import { AlertRoot } from './alert/AlertRoot';
import { WindowControls } from './components/WindowControls';
import { TooltipLayer } from './components/TooltipLayer';
import { CursorFx } from './components/CursorFx';
import { getNoticeAgreed, setTrayHue } from './lib/commands';
import { getAccent, getTheme, applyAccent, applyTheme, onSystemChange } from './lib/theme';

/** 窗口 label 分支：main = 设置主窗口（含告知页），alert = 拦截弹窗。 */
export default function App() {
  const label = getCurrentWindow().label;
  if (label === 'alert') {
    return <AlertRoot />;
  }
  return <MainWindow />;
}

function MainWindow() {
  // 三态：null = 尚未取回。**不能在未知时就渲染设置页**——那正是「未确认却可用」
  // 的漏洞；也不能默认 false，否则已同意的用户每次从托盘打开都会闪一下告知页。
  const [agreed, setAgreed] = useState<boolean | null>(null);
  const [cursorFx, setCursorFx] = useState(
    () => localStorage.getItem('main-ui:cursorfx') !== 'off',
  );

  // 同意状态的权威副本在后端（不是 localStorage）：主窗口静默启动时后端要据此
  // 决定是否 show，前端只跟随。取回失败按未同意处理（保守方向，宁可多弹一次）。
  useEffect(() => {
    let alive = true;
    void getNoticeAgreed()
      .then((v) => alive && setAgreed(v))
      .catch(() => alive && setAgreed(false));
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    const theme = getTheme();
    const accent = getAccent();
    applyTheme(theme);
    applyAccent(accent);
    // 托盘图标色相同步（浏览器 dev 环境无 Tauri，静默失败）
    void setTrayHue(accent.hue).catch(() => {});
    return theme === 'system' ? onSystemChange(() => applyTheme(theme)) : undefined;
  }, []);

  // 设置页开关自定义指针 → 全局挂载/卸载
  useEffect(() => {
    const onToggle = (e: Event) => setCursorFx((e as CustomEvent<boolean>).detail);
    window.addEventListener('main:cursorfx', onToggle);
    return () => window.removeEventListener('main:cursorfx', onToggle);
  }, []);

  return (
    <div className="flex h-full flex-col bg-[var(--window-bg)] text-[var(--text-primary)]">
      {/* 融入式顶栏：与左侧导航同底色（Win11 设置式分层），纯拖拽区 + 控制。
          Mac 式控制点居右（关闭点在最外侧）。 */}
      <header
        className="flex h-11 shrink-0 items-center justify-end bg-[var(--sidebar-bg)] pl-3 pr-5"
        data-tauri-drag-region
      >
        <div className="flex-1" data-tauri-drag-region />
        <WindowControls />
      </header>
      <div className="min-h-0 flex-1">
        {/* agreed === null：状态未知，留白而非渲染设置页（合规门控不允许抢跑） */}
        {agreed === null ? null : agreed ? (
          <SettingsRoot />
        ) : (
          <NoticePage onAgree={() => setAgreed(true)} />
        )}
      </div>
      {cursorFx && <CursorFx />}
      <TooltipLayer />
    </div>
  );
}
