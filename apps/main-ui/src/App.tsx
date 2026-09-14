import { useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SettingsRoot } from './settings/SettingsRoot';
import { NoticePage } from './notice/NoticePage';
import { AlertRoot } from './alert/AlertRoot';
import { WindowControls } from './components/WindowControls';
import { hasAgreedNotice } from './lib/commands';

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

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <header
        className="flex h-10 shrink-0 select-none items-center border-b border-border/60 bg-card pl-4"
        data-tauri-drag-region
      >
        <span className="text-xs text-muted-foreground" data-tauri-drag-region>
          危信设置
        </span>
        <div className="flex-1" />
        <WindowControls />
      </header>
      <div className="min-h-0 flex-1">
        {agreed ? <SettingsRoot /> : <NoticePage onAgree={() => setAgreed(true)} />}
      </div>
    </div>
  );
}
