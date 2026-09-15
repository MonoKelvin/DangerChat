import { useState } from 'react';
import { agreeNotice } from '../lib/commands';

/**
 * 首次启动告知页（FR-UI-08，合规文档 §5.1 强制项）。
 * 三块内容：①做什么/不做什么 ②协议风险原文 ③自行验证方式。
 * 勾选同意前功能不可用；同意后持久化不再弹出。
 */
export function NoticePage({ onAgree }: { onAgree: () => void }) {
  const [checked, setChecked] = useState(false);

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto p-8">
      <div className="max-w-2xl space-y-5">
        <div>
          <h1 className="text-xl font-semibold">欢迎使用危信</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            开始使用前，请花一分钟了解它的工作方式与风险。
          </p>
        </div>

        {/* ① 做什么/不做什么 */}
        <section className="rounded-lg border border-border/50 bg-card/60 p-4">
          <h2 className="mb-2 text-sm font-semibold">它做什么、不做什么</h2>
          <p className="text-[13px] leading-relaxed text-foreground/90">
            危信通过<b>截取你的屏幕画面并在本机识别文字</b>来工作。
          </p>
          <p className="mt-2 text-[13px] font-medium">它不会：</p>
          <ul className="mt-1 space-y-1 text-[13px] text-muted-foreground">
            <li>· 注入或修改微信程序</li>
            <li>· 读取或解密微信的聊天记录文件</li>
            <li>· 替你发送任何消息</li>
            <li>· 把你的聊天内容上传到任何服务器（v1.0 为纯本地方案）</li>
          </ul>
          <p className="mt-2 text-[13px] leading-relaxed">
            它只会：<b>阻止你自己的按键到达微信，然后提醒你。</b>
          </p>
        </section>

        {/* ② 协议风险（原文照贴） */}
        <section className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-4">
          <h2 className="mb-2 text-sm font-semibold">协议风险（请知悉）</h2>
          <p className="text-[13px] leading-relaxed text-foreground/90">
            微信《软件许可及服务协议》8.2.1.6 与《微信个人账号使用规范》1.2.6 规定，禁止
            “通过非腾讯开发、授权的第三方软件……对微信软件及其组件、模块、界面、数据等进行访问、读取或控制”。
          </p>
          <p className="mt-2 text-[13px] font-medium text-amber-600 dark:text-amber-400">
            使用本类工具可能违反微信条款，存在账号被限制或封禁的风险，责任由使用者自行承担。
          </p>
          <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">
            我们已尽力把风险降到最低：不接触微信进程、不接触微信文件、不自动发送任何消息——这些正是微信实际处罚的行为。但我们无法保证腾讯不会做出不同认定。
          </p>
        </section>

        {/* ③ 可自行验证 */}
        <section className="rounded-lg border border-border/50 bg-card/60 p-4">
          <h2 className="mb-2 text-sm font-semibold">你可以自行验证</h2>
          <ul className="space-y-1.5 text-[13px] text-muted-foreground">
            <li>· 用 Wireshark 抓包——默认配置下危信不发起任何网络连接</li>
            <li>· 用 Process Monitor 观察——危信从不读取微信目录、从不访问微信进程内存</li>
            <li>· 源码完全公开，可自行审计与构建</li>
          </ul>
        </section>

        <div className="flex items-center justify-between pt-2">
          <label className="flex cursor-pointer items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={checked}
              onChange={(e) => setChecked(e.target.checked)}
              className="size-4 accent-[var(--primary)]"
            />
            我已阅读并理解以上内容，自行承担使用风险
          </label>
          <button
            className="h-9 rounded-md bg-primary px-6 text-sm text-primary-foreground transition-opacity disabled:opacity-40"
            disabled={!checked}
            onClick={() => {
              agreeNotice();
              onAgree();
            }}
          >
            开始使用
          </button>
        </div>
      </div>
    </div>
  );
}
