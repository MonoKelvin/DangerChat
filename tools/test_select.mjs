#!/usr/bin/env node
// 增量选测（pnpm test:fast）：只跑「受未提交改动影响」的测试，排除黄金层。
//
// 变更集 = git diff --name-only <DC_TEST_BASE|HEAD> + 未跟踪文件。
//   Rust 侧：变更文件映射到 workspace 成员 → 沿依赖图算反向闭包 →
//            cargo nextest run --workspace -E 'rdeps(...) and not binary(golden_)'
//            （用 --workspace 保持 feature 统一图不变，绕开 -p 单包选测的
//              重编译坑——实测单包比全 workspace 更慢且污染缓存）
//   前端侧：apps/main → vitest main；apps/uitag → vitest uitag；
//            crates/dc-bridge → main 契约 fixtures 双写检测也要跑
//
// 退出码：任一测试失败即非零；nextest 缺失时直接报错退出（纯 nextest 路线）。

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const ROOT = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');

function sh(cmd, args) {
  return execFileSync(cmd, args, { encoding: 'utf8', cwd: ROOT }).trim();
}

// ---- 1) 变更集 ------------------------------------------------------------

const base = process.env.DC_TEST_BASE || 'HEAD';
let changed;
try {
  // core.quotepath=false：中文等非 ASCII 路径输出原始 UTF-8 而非八进制转义
  changed = [
    ...sh('git', ['-c', 'core.quotepath=false', 'diff', '--name-only', base]).split('\n'),
    ...sh('git', ['-c', 'core.quotepath=false', 'ls-files', '--others', '--exclude-standard'])
      .split('\n'),
  ]
    .filter((l) => l.length > 0)
    .map((l) => l.replaceAll('\\', '/'));
} catch (e) {
  console.error(`读取变更集失败（${base}）：${e.message}`);
  process.exit(1);
}

if (changed.length === 0) {
  console.log('无未提交改动（对比 ' + base + '）—— 没有需要跑的测试。');
  process.exit(0);
}

console.log(`变更文件 ${changed.length} 个（对比 ${base}）：`);
for (const f of changed) console.log(`  ${f}`);

// ---- 2) Rust 侧：变更包 → 反向依赖闭包 ------------------------------------

// cargo metadata 的 workspace_members 是包 id（path + name + version），
// manifest_path 字段直接给出磁盘路径，用来归属变更文件。
const meta = JSON.parse(sh('cargo', ['metadata', '--format-version', '1', '--no-deps']));
const members = meta.packages.filter((p) => meta.workspace_members.includes(p.id));

const changedRustPkgs = new Set();
for (const pkg of members) {
  const dir = pkg.manifest_path.replaceAll('\\', '/').replace(/\/Cargo\.toml$/, '');
  if (changed.some((f) => f === dir.slice(ROOT.length) || f.startsWith(dir.slice(ROOT.length) + '/'))) {
    changedRustPkgs.add(pkg.name);
  }
}

// nextest 的 rdeps(pkg) = 「包 pkg 的测试」+「（直接或间接）依赖 pkg 的包的测试」，
// 已含自身（nextest 文档：rdeps 递归包含 package 自身），无需再 + package(x)。
if (changedRustPkgs.size > 0) {
  console.log(`\nRust 变更包：${[...changedRustPkgs].join(', ')}`);
  // 全 workspace 需要重编测试二进制的包也会被 nextest 计入 rdeps 影响面；
  // 但跨包依赖发生在 workspace 内部时（如 dc-core 改动 → dc-pipeline 重编），
  // 下游包的测试也应跑 —— rdeps 正是这个语义。
  // 注意括号：nextest 过滤式里 `-` 优先级高于 `+`，不括起来减法只作用于最后一个并集项
  const expr =
    '(' + [...changedRustPkgs].map((p) => `rdeps(${p})`).join(' + ') + ')' +
    ' - binary(golden_layout) - binary(golden_ocr)';
  console.log(`nextest 过滤式：${expr}\n`);
  try {
    execFileSync('cargo', ['nextest', 'run', '--workspace', '-E', expr], {
      stdio: 'inherit',
      cwd: ROOT,
    });
  } catch {
    process.exit(1);
  }
} else {
  console.log('\n无 Rust workspace 成员变更，跳过 Rust 测试。');
}

// ---- 3) 前端侧 ------------------------------------------------------------

const frontendSuites = new Set();
if (changed.some((f) => f.startsWith('apps/main/'))) frontendSuites.add('main');
if (changed.some((f) => f.startsWith('apps/uitag/'))) frontendSuites.add('uitag');
// 契约 fixtures 双写检测：dc-bridge 的 DTO 改动需要 main 侧对照 fixtures
if (changedRustPkgs.has('dc-bridge')) frontendSuites.add('main');

for (const suite of frontendSuites) {
  console.log(`\n前端套件 ${suite}：`);
  try {
    // Windows 上 pnpm 是 .cmd 垫片，Node 的 execFileSync 不再自动解析（CVE 修复后），
    // 必须经 shell 调用；suite 取值固定为 main/uitag，拼字符串无注入面
    execFileSync(`pnpm -F ${suite} test`, {
      stdio: 'inherit',
      cwd: ROOT,
      shell: true,
    });
  } catch {
    process.exit(1);
  }
}

if (frontendSuites.size === 0) console.log('\n无前端变更，跳过 Vitest。');
console.log('\n增量测试完成（黄金层/真实桌面用例未跑：pnpm test / pnpm test:golden / pnpm test:manual）');
