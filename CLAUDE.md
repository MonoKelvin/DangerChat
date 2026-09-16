# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概览

危信（DangerChat）v1.0：Windows-only Tauri 2 桌面应用，监控目标聊天软件窗口（截图 → 区域划分 → OCR → 语义判定），在用户发送敏感/违禁内容前弹窗拦截。Rust workspace + pnpm monorepo。权威设计文档：`docs/危信v1.0_开发设计文档.md`（章节号 §x.x 常出现在代码注释里）。

## 常用命令

```bash
pnpm dev            # 主程序 tauri dev（apps/main + src-tauri）
pnpm dev:uitag      # 打标记工具 tauri dev（apps/uitag）
pnpm build          # 发布构建（安装包 ≤ 80MB 是发布指标）

pnpm test           # 全量：cargo nextest run --workspace + 前端 vitest
pnpm test:fast      # 增量选测（tools/test_select.mjs，日常改完代码用这个）
pnpm test:golden    # 黄金层（golden_layout + golden_ocr，需真实模型权重）
pnpm test:manual    # ignored 层（真实桌面/真实权重，4 个手工测试）

cargo build -p dangerchat --no-default-features   # 快速编译检查主程序壳
pnpm -F main test   # 单独跑主前端 vitest
```

### 测试纪律（重要）

- 日常验证用 `pnpm test:fast`，**不要**裸 `cargo test`（无效 doctest 装配 91s）。
- **禁止 `cargo test -p <crate>` 单包选测**：改变 feature 统一图导致全量重编译，比跑全 workspace 更慢且污染缓存。选测一律走 `cargo nextest run --workspace -E '...'` 过滤。
- 依赖真实模型权重的测试放独立 `tests/golden_*.rs` 目标（文件名即过滤键 `binary(golden_*)`）。
- nextest 过滤式里 `-` 优先级高于 `+`，排除并集要加括号：`-E "binary(golden_layout) + binary(golden_ocr)"` 要写成 `not (a + b)` 形式。
- 首次增量跑 dc-core 相关改动 3~4 分钟是编译（下游全要重编），不是测试慢。

## 架构

### Crate 分层（依赖方向自上而下，tauri 不进 dc-core/pipeline/sys）

```
src-tauri (dangerchat 主程序壳：bootstrap 装配 / 托盘 / 窗口状态 / 单实例锁)
 └─ dc-bridge（Tauri 通信层：AppState 聚合根 / 11 条 command / 事件 / AlertBus 泵线程 / 窗口发现状态机）
     └─ dc-pipeline（核心流水线）
         ├─ intercept：消息源（键盘钩子 + 草稿纪元 draft epoch，发送路径零等待）
         ├─ capture：截图
         ├─ layout：区域划分（tags.json 与 uitag 共用）
         ├─ ocr：det→cls→rec 三件套（rapidocr-core）
         ├─ sem：L1 规则（aho-corasick）/ 联系人画像 / 场景 / BGE 语义
         └─ guard：编排器（双速流水线）
         └─ dc-sys（Windows API 封装，real / mock 双实现）
              └─ dc-core（配置中心 / 日志 / 插件注册 / 模型管理）
```

- `apps/main`：主界面（React 19 + zustand + Tailwind 4，设置/弹窗/通知）。
- `apps/uitag`：打标记工具（独立 Tauri app，产出 tags.json）。
- `spikes/m3-inference`、`tools/redline_scan`、`tools/training`：实验/离线工具，不进发布物。

### 关键技术约束（写代码前必知）

- **并发模型（ADR-09）：std::thread + crossbeam-channel，不引入 tokio**。判定槽位用 arc-swap 无锁交换（§2.2 verdict_slot）。
- **推理栈**：ort 2.0.0-rc.13 + DirectML EP（仅 Windows，directml 全平台开启）；分词用 tokenizers（fancy-regex，不引 onig C 绑定）。
- 双速流水线 + 感知预算：热路径（按键路径）上有硬性耗时约束，见设计文档 §2.1–2.4。代码注释里的 §x.x 引用即指该文档。

### Tauri 已知坑（排查过的真实事故）

1. **`app.manage(Arc<T>)` 必须配 `State<'_, Arc<T>>`**——Tauri 2.11 是纯 TypeId 查找，不自动解包 Arc。不匹配时所有命令抛 "state not managed"，而前端 catch 是静默的，表现为"功能不生效"但无报错。新增命令照抄 `crates/dc-bridge/src/commands.rs` 现有签名。
2. **`cargo check` 后 `tauri dev` 可能跑旧 exe**（check 不产出 exe，dev 误判无需重编）。改完 Rust 先确认 `cargo build -p dangerchat --no-default-features` 输出有 `Compiling` 行再起 dev。
3. 单实例锁 + 重启接力：新实例带 `--wait-for-pid` 等旧进程释放 Mutex（见 `src-tauri/src/lib.rs` 注释），改动启动流程时别破坏这个机制。

### 数据目录

- 默认 `%APPDATA%\com.monostudio.dangerchat\`（bootstrap.json / config.json / rules.json / contacts.json / scenes.json / logs/ / models/ / stats.json）。
- 用户自定义目录：默认目录下 `bootstrap.json` 管理数据目录指针，bootstrap 启动时解析，重启生效。
- rules.json 热重载；models/ 与 datasets/ 有目录监听（自助训练数据流）。

### 版本与元数据（单一来源，改前必读）

- **版本号**：前端与打包的唯一来源是根 `package.json` 的 `version`（两个 `tauri.conf.json` 用 `"version": "<path>"` 引用它，前端经 `src/lib/meta.ts` 读取）。**Cargo 无法引用 package.json**——根 `Cargo.toml` 的 `[workspace.package].version` 是不可避免的第二处，**升版本时两处必须同时改**。
- **应用名**：主程序 = `src-tauri/tauri.conf.json` 的 `productName`；打标工具 = `apps/uitag/src-tauri/tauri.conf.json` 的 `productName`。前端经各自 `src/lib/meta.ts` 的 `APP_NAME` 引用，Rust 侧托盘 tooltip 也从 tauri config 读取——组件/命令里不要写字面量。
- **作者 / 仓库 / 协议**：根 `package.json` 的 `author` / `repository` / `license`，前端经 `meta.ts` 派生（`REPO_URL` / `AUTHOR_URL` / `LICENSE`），不要在代码里写死这些 URL。

### 构建 profile 注意

- `[profile.dev.package."*"]` opt-level=3：依赖包（尤其 imageproc 的 NCC 模板匹配）按 release 编译，否则 tauri dev 下预标注传播慢 10~50× 会卡死。自编译代码保持默认以保留增量速度。

## 仓库约定

- 注释、commit message、文档均为中文；commit 风格参照 git log（"M8：xxx"、"M7 加固：xxx" 里程碑式）。
- release profile 为体积优化（opt-level "z" / lto thin / panic abort / strip），对应 NFR-11。
