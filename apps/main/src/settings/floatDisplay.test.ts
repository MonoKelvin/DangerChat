import { describe, expect, it } from 'vitest';
import { formatByStep } from '../components/NumberInput';

/**
 * UT-UI-05 伴随：数值控件显示精度。
 *
 * 背景：基线阈值（sem.threshold.*）在后端是 f32 常量，经 f64 宽化 + JSON 往返后
 * 会变成 0.3499999940395355 这类值。控件若直接 String(value) 就会显示 10+ 位尾数。
 * 显示必须按 schema 步长（float → 0.05 → 两位）截断，但存储值不得被改写。
 */
describe('数值显示精度（UT-UI-05）', () => {
  it('f32 宽化尾数被截到步长小数位', () => {
    expect(formatByStep(0.3499999940395355, 0.05)).toBe('0.35');
    expect(formatByStep(0.550000011920929, 0.05)).toBe('0.55');
    expect(formatByStep(0.44999998807907104, 0.05)).toBe('0.45');
  });

  it('整数步长不出现小数点', () => {
    expect(formatByStep(2000, 1)).toBe('2000');
    expect(formatByStep(9.999, 1)).toBe('10');
  });

  it('非有限值显示为空串（避免 NaN/Infinity 字样进入输入框）', () => {
    expect(formatByStep(Number.NaN, 0.05)).toBe('');
    expect(formatByStep(Number.POSITIVE_INFINITY, 0.05)).toBe('');
  });

  it('格式化是纯显示层：原值不变', () => {
    const raw = 0.3499999940395355;
    const shown = Number(formatByStep(raw, 0.05));
    // 显示值可解析回 0.35（两位），但原值未被就地改写
    expect(shown).toBe(0.35);
    expect(raw).toBe(0.3499999940395355);
  });
});
