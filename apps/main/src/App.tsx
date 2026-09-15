import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Moon, Sun, Monitor } from 'lucide-react';
import { SettingsRoot } from './settings/SettingsRoot';
import { NoticePage } from './notice/NoticePage';
import { AlertRoot } from './alert/AlertRoot';
import { WindowControls } from './components/WindowControls';
import { IconButton } from './components/IconButton';
import { hasAgreedNotice } from './lib/commands';
import { getTheme, setTheme, nextTheme, onSystemChange, applyTheme, type Theme } from './lib/theme';

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
  const [theme, setThemeState] = useState<Theme>(getTheme());

  useEffect(() => {
    applyTheme(theme);
    const cleanup = theme === 'system' ? onSystemChange(() => applyTheme(theme)) : undefined;
    return cleanup;
  }, [theme]);

  const cycleTheme = () => {
    const next = nextTheme(theme);
    setTheme(next);
    setThemeState(next);
  };

  const ThemeIcon = theme === 'dark' ? Moon : theme === 'light' ? Sun : Monitor;

  return (
    <div className="flex h-full flex-col bg-[var(--window-bg)] text-[var(--text-primary)]">
      {/* 融入式顶栏：无边框，纯拖拽区 + 控制 */}
      <header className="flex h-12 shrink-0 items-center justify-between px-4" data-tauri-drag-region>
        <div className="flex-1" data-tauri-drag-region />
        <div className="flex items-center gap-0.5">
          <IconButton
            size="sm"
            variant="ghost"
            title={`主题: ${theme === 'dark' ? '深色' : theme === 'light' ? '浅色' : '跟随系统'}`}
            onClick={cycleTheme}
          >
            <ThemeIcon className="size-[18px]" strokeWidth={1.5} />
          </IconButton>
          <WindowControls />
        </div>
      </header>
      <div className="min-h-0 flex-1">
        {agreed ? <SettingsRoot /> : <NoticePage onAgree={() => setAgreed(true)} />}
      </div>
    </div>
  );
}
