import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { NoticePage } from './NoticePage';

/** UT-UI-05：首次启动告知页——未勾选不可用、勾选后触发同意并持久化。 */
describe('NoticePage（UT-UI-05）', () => {
  it('未勾选时「开始使用」禁用；勾选后可点并回调', async () => {
    localStorage.removeItem('dangerchat:notice-agreed');
    let agreed = false;
    render(<NoticePage onAgree={() => (agreed = true)} />);

    const btn = screen.getByRole('button', { name: /开始使用/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);

    const cb = screen.getByRole('checkbox') as HTMLInputElement;
    fireEvent.click(cb);
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    await waitFor(() => expect(agreed).toBe(true));
    expect(localStorage.getItem('dangerchat:notice-agreed')).toBe('1');
  });

  it('三块内容齐全（做什么/风险/验证）', () => {
    localStorage.removeItem('dangerchat:notice-agreed');
    render(<NoticePage onAgree={() => {}} />);
    expect(screen.getAllByText(/它做什么、不做什么/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/协议风险/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/你可以自行验证/).length).toBeGreaterThan(0);
    // 风险原文关键词
    expect(screen.getAllByText(/自行承担/).length).toBeGreaterThan(0);
  });
});
