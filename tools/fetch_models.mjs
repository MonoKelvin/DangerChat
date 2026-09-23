#!/usr/bin/env node
// 拉取「未入库」的模型权重（pnpm install 后由 postinstall 自动触发）。
//
// 背景：所有 ONNX 权重均已随仓库入库（克隆即可用）。本脚本保留用于将来
// 新加入的大模型权重下载所需，但目前无需下载任何文件。
//
// 设计取向：
//   · 零依赖（Node 18+ 原生 fetch，仓库要求 Node 24）；
//   · 网络失败不阻断 install（fail-soft）——打印指引后退出 0，重跑 install 即重试；
//   · 下载到 .part 临时文件再原子重命名，中断不会留下半截文件冒充成品；
//   · HF 直连不通时用镜像（可 DC_HF_ENDPOINT 覆盖）。
//
// 手动运行 / 强制重下：
//   node tools/fetch_models.mjs           # 常规：补缺
//   DC_SKIP_MODEL_FETCH=1 pnpm install    # 跳过（离线/CI 无需模型时）
//   node tools/fetch_models.mjs --force   # 忽略已存在，全部重下

import { existsSync, statSync, mkdirSync, createWriteStream, renameSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

const ROOT = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const FORCE = process.argv.includes('--force');

// HF 端点候选：环境变量优先，其次直连，再镜像（国内直连常超时）。
const ENDPOINTS = [
  process.env.DC_HF_ENDPOINT,
  'https://huggingface.co',
  'https://hf-mirror.com',
].filter(Boolean);

// 需要拉取的「未入库」权重清单。目前所有权重均已入库，清单为空。
// 若将来新增超 100MB 权重需下载，此处补充条目。
const MODELS = [];

function fmtMB(bytes) {
  return `${(bytes / 1048576).toFixed(0)}MB`;
}

/** 已存在且大小正确 → 无需下载。 */
function isSatisfied(absPath, expectedSize) {
  if (!existsSync(absPath)) return false;
  if (!expectedSize) return true; // 未声明大小 → 只要存在即可
  try {
    return statSync(absPath).size === expectedSize;
  } catch {
    return false;
  }
}

async function download(model) {
  const abs = join(ROOT, model.dest);
  mkdirSync(dirname(abs), { recursive: true });
  const tmp = `${abs}.part`;

  let lastErr;
  for (const base of ENDPOINTS) {
    const url = `${base}/${model.repoPath}`;
    try {
      process.stdout.write(`  下载 ${model.name}（${fmtMB(model.size)}）自 ${base} … `);
      const res = await fetch(url, { redirect: 'follow' });
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      await pipeline(Readable.fromWeb(res.body), createWriteStream(tmp));

      // 完整性校验：大小不符即视为失败，删临时文件换下一端点。
      const got = statSync(tmp).size;
      if (model.size && got !== model.size) {
        rmSync(tmp, { force: true });
        throw new Error(`大小不符（期望 ${model.size}，实得 ${got}）`);
      }
      renameSync(tmp, abs);
      console.log('完成');
      return true;
    } catch (e) {
      console.log(`失败（${e.message}）`);
      rmSync(tmp, { force: true });
      lastErr = e;
    }
  }
  console.error(`  ✗ ${model.name} 全部端点均失败：${lastErr?.message ?? '未知错误'}`);
  return false;
}

async function main() {
  if (process.env.DC_SKIP_MODEL_FETCH) {
    console.log('[fetch-models] DC_SKIP_MODEL_FETCH 已设，跳过模型下载。');
    return 0;
  }

  const missing = MODELS.filter((m) => FORCE || !isSatisfied(join(ROOT, m.dest), m.size));
  if (missing.length === 0) {
    console.log('[fetch-models] 所需模型均已就位，无需下载。');
    return 0;
  }

  console.log(`[fetch-models] 需拉取 ${missing.length} 个未入库权重：`);
  let ok = true;
  for (const m of missing) {
    if (!(await download(m))) ok = false;
  }

  if (!ok) {
    // fail-soft：不阻断 install。模型缺失时 sem 走 fail-open（仅 L1 规则），程序仍可跑。
    console.error(
      '\n[fetch-models] 部分模型下载失败。程序仍可运行（语义判定降级为仅规则引擎）。\n' +
        '  联网后重跑 `pnpm install` 或 `node tools/fetch_models.mjs` 即可补齐；\n' +
        '  国内网络可设镜像：DC_HF_ENDPOINT=https://hf-mirror.com node tools/fetch_models.mjs\n' +
        '  手动下载见 resources/models/README.md。',
    );
  }
  return 0; // 永不因下载失败让 install 非零退出
}

main().then((code) => process.exit(code));
