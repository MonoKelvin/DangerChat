import { describe, expect, it } from 'vitest';
import type { RuleDto } from '../lib/types';

/**
 * UT-UI-04 伴随：违禁词编辑器的数据变换（增删改在组件内联实现，
 * 这里锁定 RuleDto 的形态约束与保存载荷的可序列化性）。
 */
describe('规则编辑器数据契约（UT-UI-04）', () => {
  it('RuleDto 往返序列化（保存载荷）', () => {
    const rules: RuleDto[] = [
      { pattern: 'sb', match: 'word', applies_to: ['formal'] },
      { pattern: '(傻|沙)(比|逼)', match: 'regex', applies_to: ['all'] },
    ];
    const json = JSON.stringify(rules);
    const back = JSON.parse(json) as RuleDto[];
    expect(back).toEqual(rules);
    expect(back[0].applies_to).toContain('formal');
  });

  it('新增规则的默认形态', () => {
    // RulesEditor add() 的默认值——与后端 RuleDef 校验兼容（substring/all 合法）
    const fresh: RuleDto = { pattern: '', match: 'substring', applies_to: ['all'] };
    expect(['word', 'substring', 'regex']).toContain(fresh.match);
  });

  it('删除：按索引过滤', () => {
    const rules: RuleDto[] = [
      { pattern: 'a', match: 'word', applies_to: ['all'] },
      { pattern: 'b', match: 'word', applies_to: ['all'] },
      { pattern: 'c', match: 'word', applies_to: ['all'] },
    ];
    const after = rules.filter((_, j) => j !== 1);
    expect(after.map((r) => r.pattern)).toEqual(['a', 'c']);
  });
});
