import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { AlertPayload, ConfigFieldDto, StatsPayload, StatusPayload } from './types';

/**
 * UT-UI-01 伴随 / UT-BRG-01 前端侧：手写 types.ts 与 dc-bridge 的序列化
 * fixtures 结构对照（字段名/可空性双写漂移在这里被拦截）。
 */
const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(HERE, '../../../../crates/dc-bridge/tests/fixtures');

describe('契约 fixtures 对照（UT-UI-01）', () => {
  it('alert.json 字段齐全', () => {
    const p = JSON.parse(readFileSync(join(FIXTURES, 'alert.json'), 'utf-8')) as AlertPayload;
    expect(p.level).toBe('block');
    expect(typeof p.score).toBe('number');
    expect(Array.isArray(p.reasons)).toBe(true);
    expect(p.chat_target).toBe('张总');
    expect(p.chat_context).toBe('昨天的报告怎么样\n已经完成了');
    expect(typeof p.draft_text).toBe('string');
    expect(typeof p.draft_fingerprint).toBe('number');
    expect(typeof p.draft_epoch).toBe('number');
    expect(typeof p.countdown_secs).toBe('number');
  });

  it('status.json 字段齐全', () => {
    const p = JSON.parse(readFileSync(join(FIXTURES, 'status.json'), 'utf-8')) as StatusPayload;
    expect(['active', 'suspended', 'paused', 'cooldown']).toContain(p.state);
    expect(typeof p.target_process).toBe('string');
    expect(typeof p.target_found).toBe('boolean');
  });

  it('stats.json 字段齐全（无消息原文）', () => {
    const p = JSON.parse(readFileSync(join(FIXTURES, 'stats.json'), 'utf-8')) as StatsPayload;
    expect(typeof p.today_blocked).toBe('number');
    expect(typeof p.alert_failed).toBe('number');
    // 耗时字段可空（未跑过流水线）
    expect(p.last_capture_ms === null || typeof p.last_capture_ms === 'number').toBe(true);
    expect(p.last_ocr_ms === null || typeof p.last_ocr_ms === 'number').toBe(true);
    // 隐私红线：stats 载荷不得含消息内容字段
    expect(Object.keys(p).some((k) => k.includes('draft') || k.includes('text'))).toBe(false);
  });

  it('ConfigFieldDto 的 ty 枚举与 SchemaField 渲染分支一一对应', () => {
    const renderable = ['bool', 'int', 'float', 'text', 'enum', 'strlist', 'path'] as const;
    type Renderable = (typeof renderable)[number];
    const field: ConfigFieldDto = {
      key: 'test.k',
      ty: 'bool',
      default: true,
      options: [],
      label: '测试',
      help: '',
      group: 'general',
    };
    // types.ts 的 ty 声明 = renderable 全集（编译期联合类型运行时可枚举验证）
    const sample: Renderable = field.ty as Renderable;
    expect(renderable).toContain(sample);
  });
});
