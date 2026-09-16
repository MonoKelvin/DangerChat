import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SettingsRoot } from './settings/SettingsRoot';
import { NoticePage } from './notice/NoticePage';
import { AlertRoot } from './alert/AlertRoot';
import { WindowControls } from './components/WindowControls';
import { TooltipLayer } from './components/TooltipLayer';
import { CursorFx } from './components/CursorFx';
import { hasAgreedNotice, setTrayHue } from './lib/commands';
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
  const [agreed, setAgreed] = useState(hasAgreedNotice());
  const [cursorFx, setCursorFx] = useState(
    () => localStorage.getItem('main-ui:cursorfx') !== 'off',
  );

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
        {agreed ? <SettingsRoot /> : <NoticePage onAgree={() => setAgreed(true)} />}
      </div>
      {cursorFx && <CursorFx />}
      <TooltipLayer />
    </div>
  );
}
