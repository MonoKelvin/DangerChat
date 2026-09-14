import { useEffect, useState } from 'react';
import { IconDeviceDesktop, IconMoon, IconSun } from '@tabler/icons-react';
import { Button } from './ui/button';
import {
  THEME_LABEL,
  getTheme,
  nextTheme,
  onSystemChange,
  setTheme,
  type Theme,
} from '../lib/theme';

const ICONS: Record<Theme, React.ReactNode> = {
  dark: <IconMoon className="size-4" />,
  light: <IconSun className="size-4" />,
  system: <IconDeviceDesktop className="size-4" />,
};

/** 主题切换：点击循环 深 → 浅 → 系统；system 模式下跟随系统实时变化。 */
export function ThemeToggle() {
  const [theme, setThemeState] = useState<Theme>(getTheme);

  // system 模式下系统偏好变化时重新应用
  useEffect(() => {
    if (theme !== 'system') return;
    return onSystemChange(() => {
      setTheme(theme); // 重新解析并应用
    });
  }, [theme]);

  const cycle = () => {
    const next = nextTheme(theme);
    setTheme(next);
    setThemeState(next);
  };

  return (
    <Button
      variant="ghost"
      size="icon"
      className="size-8 text-muted-foreground hover:shadow-sm"
      onClick={cycle}
      data-tip={`主题：${THEME_LABEL[theme]}（点击切换）`}
    >
      {ICONS[theme]}
    </Button>
  );
}
