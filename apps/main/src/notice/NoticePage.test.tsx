import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { NoticePage } from './NoticePage';

/** 同意状态与外部链接都走后端命令（非 localStorage / 非 `<a target="_blank">`），故 mock 命令层。 */
const { ack, openExternal } = vi.hoisted(() => ({
  ack: vi.fn<() => Promise<void>>(),
  openExternal: vi.fn<(url: string) => Promise<void>>(),
}));
vi.mock('../lib/commands', () => ({
  ackNotice: () => ack(),
  openExternal: (url: string) => openExternal(url),
}));

/** UT-UI-05：首次启动告知页——未勾选不可用、勾选后落盘并回调；落盘失败不得放行。 */
describe('NoticePage（UT-UI-05）', () => {
  // vitest 未开 globals，RTL 的自动 cleanup 不会注册 → 必须显式清理，
  // 否则上一个用例的 DOM 残留会让 getByRole 命中多个元素。
  afterEach(cleanup);

  beforeEach(() => {
    ack.mockReset();
    ack.mockResolvedValue(undefined);
    openExternal.mockReset();
    openExternal.mockResolvedValue(undefined);
  });

  it('协议链接经后端 openExternal 打开（WebView 不会自己交给系统浏览器）', async () => {
    render(<NoticePage onAgree={() => {}} />);

    // 回归：曾用 `<a href target="_blank">`，在 Tauri WebView 里点击毫无反应。
    // 现在必须是会触发 openExternal 的元素，且没有 href（不会被当导航处理）。
    const agreement = screen.getByRole('button', { name: /微信软件许可及服务协议/ });
    expect(agreement.getAttribute('href')).toBeNull();

    fireEvent.click(agreement);
    await waitFor(() => expect(openExternal).toHaveBeenCalledTimes(1));
    expect(openExternal).toHaveBeenCalledWith(
      'https://weixin.qq.com/cgi-bin/readtemplate?lang=zh_CN&t=weixin_agreement&s=default',
    );

    fireEvent.click(screen.getByRole('button', { name: /微信个人账号使用规范/ }));
    await waitFor(() => expect(openExternal).toHaveBeenCalledTimes(2));
    expect(openExternal).toHaveBeenLastCalledWith(
      'https://weixin.qq.com/agreement/personal_account?lang=zh_CN',
    );
  });

  it('未勾选时「开始使用」禁用；勾选后落盘成功才回调', async () => {
    let agreed = false;
    render(<NoticePage onAgree={() => (agreed = true)} />);

    const btn = screen.getByRole('button', { name: /开始使用/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(ack).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('checkbox') as HTMLInputElement);
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    await waitFor(() => expect(agreed).toBe(true));
    expect(ack).toHaveBeenCalledTimes(1);
  });

  it('落盘失败时不放行，并显示错误', async () => {
    ack.mockRejectedValue(new Error('磁盘只读'));
    let agreed = false;
    render(<NoticePage onAgree={() => (agreed = true)} />);

    fireEvent.click(screen.getByRole('checkbox') as HTMLInputElement);
    fireEvent.click(screen.getByRole('button', { name: /开始使用/ }));

    await waitFor(() => expect(screen.getByText(/确认状态保存失败/)).toBeTruthy());
    expect(agreed).toBe(false);
    // 失败后按钮恢复可点（可重试），而不是卡在 busy
    await waitFor(() =>
      expect((screen.getByRole('button', { name: /开始使用/ }) as HTMLButtonElement).disabled).toBe(
        false,
      ),
    );
  });

  it('两块内容齐全（风险/原则底线）', () => {
    render(<NoticePage onAgree={() => {}} />);
    expect(screen.getAllByText(/协议风险/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/原则与底线/).length).toBeGreaterThan(0);
    // 风险原文关键词
    expect(screen.getAllByText(/自行承担/).length).toBeGreaterThan(0);
    // 原则底线关键词
    expect(screen.getAllByText(/不注入或修改微信程序/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/绕过监管/).length).toBeGreaterThan(0);
  });
});
