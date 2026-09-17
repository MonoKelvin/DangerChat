import { useState } from 'react';
import { ackNotice } from '../lib/commands';
import { APP_NAME } from '../lib/meta';
import { Checkbox } from '../components/Checkbox';
import { ExtLink } from '../components/ExtLink';

import { WECHAT_AGREEMENT, WECHAT_PERSONAL_RULES } from '../settings/SettingsRoot';

/**
 * 首次启动告知页（FR-UI-08，合规文档 §5.1 强制项）。
 * 两块内容：①协议风险 ②原则与底线（照贴关于页）。
 * 勾选同意前功能不可用；同意后持久化不再弹出。
 *
 * 「同意」是一次**落盘写入**（后端 `shell.notice_agreed`）。写失败不得放行——
 * 界面若进了设置页而后端仍认为未同意，拦截会被门控关掉而用户毫无察觉。
 */
export function NoticePage({ onAgree }: { onAgree: () => void }) {
  const [checked, setChecked] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await ackNotice();
      onAgree();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full items-start justify-center overflow-y-auto p-8 pt-12">
      <div className="max-w-2xl space-y-6">
        <div>
          <h1 className="text-xl font-semibold">欢迎使用 {APP_NAME}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            使用前请务必仔细阅读以下内容。
          </p>
        </div>

        {/* 协议风险（原文照贴） */}
        <section className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-5 py-4">
          <h2 className="mb-2 text-sm font-semibold">协议风险</h2>
          <p className="text-[14px] leading-relaxed text-foreground/90">
            微信《软件许可及服务协议》8.2.1.6 与《微信个人账号使用规范》1.2.6 规定，禁止
            “通过非腾讯开发、授权的第三方软件……对微信软件及其组件、模块、界面、数据等进行访问、读取或控制”。
          </p>
          <p className="mt-2 text-[14px] font-medium text-amber-600 dark:text-amber-400">
            使用本类工具可能违反微信条款，存在账号被限制或封禁的风险，责任由使用者自行承担。
          </p>
          <p className="mt-2 text-[14px] leading-relaxed text-muted-foreground">
            本软件仅在本地屏幕画面与按键层作判断，从不接触微信进程、从不读取微信文件、从不发送任何消息。
            但我们无法保证腾讯不会做出不同认定。使用前请阅读
            <ExtLink url={WECHAT_AGREEMENT}>《微信软件许可及服务协议》</ExtLink>
            与
            <ExtLink url={WECHAT_PERSONAL_RULES}>《微信个人账号使用规范》</ExtLink>
            。
          </p>
        </section>

        {/* 原则与底线（照贴关于页） */}
        <section className="rounded-lg border border-border/50 bg-card/60 px-5 py-4">
          <h2 className="mb-2 text-sm font-semibold">原则与底线</h2>
          <ul className="space-y-1.5 text-[14px] leading-relaxed text-foreground/90">
            <li>· 不注入或修改微信程序</li>
            <li>· 不读取或解密微信聊天记录文件</li>
            <li>· 不替你发送任何消息</li>
            <li>· 不把聊天内容上传到任何服务器</li>
            <li>· 绝不触碰法律法规底线：不开发、不内置任何绕过监管或对抗审查的功能</li>
            <li>· 绝不采集与拦截无关的数据：识别仅在内存中进行，落盘内容不包含消息原文</li>
          </ul>
        </section>

        <div className="flex items-center justify-between gap-4 pt-2">
          <Checkbox
            checked={checked}
            onChange={setChecked}
            label="我已阅读并理解以上内容，自行承担使用风险"
          />
          <button
            className="h-9 shrink-0 rounded-lg bg-[var(--brand)] px-6 text-sm font-medium text-[var(--brand-text)] shadow-[var(--shadow-sm)] transition-colors hover:bg-[var(--brand-hover)] disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--focus-ring)]"
            disabled={!checked || busy}
            onClick={() => void confirm()}
          >
            {busy ? '正在保存…' : '开始使用'}
          </button>
        </div>

        {/* 落盘失败即不放行：用户必须看到「没生效」，而不是进了设置页却发现拦截没跑 */}
        {error && (
          <p className="text-sm text-[var(--danger)]">
            确认状态保存失败，请重试：{error}
          </p>
        )}
      </div>
    </div>
  );
}
