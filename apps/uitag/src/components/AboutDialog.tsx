import { useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { IconExternalLink, IconInfoCircle } from '@tabler/icons-react';
import { Button } from './ui/button';

const VERSION = '0.1.0';
const RELEASE_DATE = '2026-09-15';
const REPO_URL = 'https://github.com/MonoKelvin/DangerChat';
const AUTHOR_URL = 'https://github.com/MonoKelvin';

/** 关于弹窗：顶栏触发；窗口居中悬浮卡片，与快捷键帮助同款形态。
 *  版本号与 tauri.conf.json / Cargo.toml 同步维护。 */
export function AboutDialog() {
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open]);

  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        className="size-8 text-muted-foreground hover:shadow-sm"
        onClick={() => setOpen(true)}
        data-tip="关于"
      >
        <IconInfoCircle className="size-4" />
      </Button>

      {open && (
        <>
          {/* 透明点击层：点外部关闭，不遮挡背景 */}
          <div className="fixed inset-0 z-40" onPointerDown={close} />
          <div
            role="dialog"
            className="animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-4 fixed top-1/2 left-1/2 z-50 w-80 -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border/70 bg-popover/75 p-6 text-popover-foreground shadow-2xl shadow-black/60 backdrop-blur-2xl outline-none"
          >
            <div className="flex flex-col items-center text-center">
              <img src="app-icon.png" alt="UiTag" className="size-20 rounded-2xl shadow-md" />
              <h2 className="mt-4 text-base font-semibold tracking-tight">
                UiTag <span className="ml-1 font-mono text-sm text-muted-foreground">v{VERSION}</span>
              </h2>
              <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                DangerChat 项目的截图打标工具：框选、标注、多选比对与导出。
              </p>
            </div>

            <dl className="mt-5 space-y-2 border-t border-border/60 pt-4 text-xs">
              <div className="flex justify-between gap-4">
                <dt className="text-muted-foreground">发布时间</dt>
                <dd className="tabular-nums">{RELEASE_DATE}</dd>
              </div>
              <div className="flex justify-between gap-4">
                <dt className="text-muted-foreground">作者</dt>
                <dd>
                  <button
                    type="button"
                    className="text-primary hover:underline"
                    onClick={() => void openUrl(AUTHOR_URL)}
                  >
                    Mono Kelvin
                  </button>
                </dd>
              </div>
              <div className="flex justify-between gap-4">
                <dt className="text-muted-foreground">版权协议</dt>
                <dd>MIT License</dd>
              </div>
              <div className="flex justify-between gap-4">
                <dt className="text-muted-foreground">源码</dt>
                <dd>
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 text-primary hover:underline"
                    onClick={() => void openUrl(REPO_URL)}
                  >
                    GitHub
                    <IconExternalLink className="size-3" />
                  </button>
                </dd>
              </div>
            </dl>

            <p className="mt-4 text-center text-[10px] text-muted-foreground/70">
              © 2026 Mono Kelvin · DangerChat
            </p>
          </div>
        </>
      )}
    </>
  );
}
