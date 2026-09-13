# 危信 v1.0 — M3 推理基座 spike 报告

- 版本：v1.0-spike.2（补真实模型复测与 §5.7 判别力发现）
- 日期：2026-09-13
- 状态：已评审（§7 三项决策全部拍板，落档见设计文档 §12.1 第 6~8 条、ADR-14/15）
- 关联文档：《危信v1.0_开发设计文档.md》§2/§5.5/§5.6/§5.7/§8/§10、《合规与风险说明.md》
- DoD（§10 M3）：spike 报告（黄金图输出、选型结论、双端耗时、int8 vs fp32 准确率对照）
- 本报告覆盖：选型结论 + 双端耗时（代理与**真实模型**）+ **int8 vs fp32 对照**（已完成）；
  黄金图输出（待 M7 标注 → M4 训练，见 §6）

---

## 1. 结论摘要

| # | 结论 | 影响 |
|---|---|---|
| 1 | `ort 2.0.0-rc.13` 接入成功，**ONNX Runtime 1.28 预编译库与 `DirectML.dll` 由构建脚本自动就位**，无需手工搬运 | 消除设计文档风险表「ort 需随附 ONNX Runtime 动态库」这一项 |
| 2 | DirectML EP 在本机（RTX 3060 Laptop + Intel UHD）**可用**，无需 CUDA 环境 | ADR-05 成立，A/N/I 显卡通吃 |
| 3 | 真实算量级下 **DirectML 比 CPU 快 10～25 倍**，全部远优于 §9 预算（YOLO 级 CPU 35ms vs 预算 300ms） | §9 性能预算有余量；CPU 路径是可用兜底而非勉强达标 |
| 4 | **小模型（< 0.1 GFLOPs）CPU 反而更快**（0.047ms vs 0.173ms）：EP 派发开销占主导 | ⚠️ **需修订文档**：设备策略要按模块决定，不能只有一个全局 `device.prefer_gpu`（见 §4.1） |
| 5 | DirectML 会话初始化：进程内首个 **350～1060ms**、同进程重建 **36～73ms**；真实 det 模型含首跑的「可服务」就绪约 1.5s | §5.8「挂起卸载 → 唤醒重载 < 500ms」**在代理模型下成立**，但真实模型更贵（见 §3.5 修正）；fail-open 空窗需量化写出（§4.2） |
| 6 | OCR/YOLO 候选 crate 均建立在**同一个 `ort v2.0.0-rc.13`** 之上，无版本冲突，只需一份 ONNX Runtime | 选型可落地，见 §5 |
| 7 | 真实模型（bge-small int8 / PP-OCRv4 mobile）实测：**体积与耗时全部达标**（sem 24MB + 2.8ms；OCR 三件套 16.2MB，det DML 25.8ms） | FR-SEM-05 常驻 ~25MB 实测成立；OCR det 必须 GPU（§3.5） |
| 8 | ⚠️ **§5.7 的模板相似度判别力 ≈ 随机（55%）**，但嵌入空间线性可分（探针 80%）；int8 与 fp32 判别力相同 | **必须修订 §5.7 L2** 为「嵌入 + 预训练线性头」（§4.3）；ADR-10（int8）维持成立 |

---

## 2. 环境

| 项 | 值 |
|---|---|
| CPU | 11th Gen Intel Core i7-11800H @ 2.30GHz（8C16T） |
| 内存 | 16 GB |
| GPU | NVIDIA GeForce RTX 3060 Laptop 6GB（驱动 592.82） + Intel UHD Graphics |
| OS | Windows（`x86_64-pc-windows-msvc`） |
| ONNX Runtime | 1.28（rel-1.28.0，commit da9b5e3，Release build） |
| Rust crate | `ort 2.0.0-rc.13`（features: `directml`；默认含 `download-binaries`/`copy-dylibs`/`api-27`） |
| 构建 profile | `--release`（除非注明） |

---

## 3. 实测数据

### 3.1 执行提供器

```
DirectML::is_available() = true
CPU::is_available()      = true
```

会话创建采用 **DirectML 优先 + CPU 兜底** 的 EP 链（`with_execution_providers([DirectML, CPU])`），
符合 §2.4「GPU EP 初始化失败自动回退 CPU」。

### 3.2 双端耗时（代理模型，release，intra-op 线程 = 4）

代理模型由 `tools/training/make_bench_models.py` 生成：算子类别与目标模型一致（Conv/Relu/Pool/MatMul），
算量对齐（详见表中 GFLOPs）。**这是「同算量」而非「同模型」，M4 拿到真实权重后必须复测。**

| 用途 | 代理模型 | 规模 | EP | init | warmup | **p50** | p99 | §9 预算（CPU/GPU） | 判定 |
|---|---|---|---|---|---|---|---|---|---|
| layout（YOLOv8n@640） | `yolo-proxy-640` | 10.56 GFLOPs | CPU | 65.6ms | 41.1ms | **35.39ms** | 38.88ms | 300 / 80ms | ✅ |
| | | | DirectML | 371.8ms | 10.2ms | **3.30ms** | 3.52ms | | ✅（快 10.7×） |
| ocr det（960×960） | `ocr-det-proxy-960` | 15.26 GFLOPs | CPU | 59.2ms | 55.3ms | **59.09ms** | 60.62ms | — | ✅ |
| | | | DirectML | 353.6ms | 11.4ms | **6.01ms** | 6.25ms | | ✅（快 9.8×） |
| ocr rec（48×320） | `ocr-rec-proxy` | 7.39 GFLOPs | CPU | 49.7ms | 27.4ms | **25.10ms** | 30.01ms | 120 / 50ms | ✅ |
| | | | DirectML | 358.6ms | 3.4ms | **1.02ms** | 1.12ms | | ✅（快 24.5×） |
| sem（句向量，512 tok） | `sem-proxy-int64` | 0.91 GFLOPs | CPU | 77.3ms | 4.8ms | **4.94ms** | 5.37ms | 150 / 30ms | ✅ |
| | | | DirectML | 375.3ms | 1.9ms | **0.43ms** | 0.49ms | | ✅ |
| 对照（极小模型） | `mnist-8` | ~0.05 GFLOPs | CPU | 54.8ms | 0.3ms | **0.047ms** | 0.192ms | — | **CPU 更快** |
| | | | DirectML | 368.3ms | 3.7ms | 0.173ms | 0.716ms | | ⚠️ 派发开销占主导 |

### 3.3 会话初始化与重载成本（架构关键）

同进程内连续创建多个会话的耗时（`--sessions 3`）：

| 模型 | CPU | DirectML |
|---|---|---|
| `yolo-proxy-640` | 65.6ms → **16.4ms** | 371.8ms → **49.0ms** |
| `sem-proxy-int64` | 77.3ms → **29.9ms** | 375.3ms → **73.1ms** |

- DirectML 首个会话 = 设备/驱动初始化（一次性，约 0.3s）+ 建图；后续会话仅建图。
- **冷启动**（首次运行该进程、驱动冷）实测过一次 **2469ms** 的极端值，属一次性现象，不影响稳态。
- 结论：§5.8「唤醒后按需重载（小模型 < 500ms）」在**同进程重建**语义下成立（GPU 最差 73ms）；
  但**进程启动后 GPU 首次就绪需 0.35～1.06s**，这段期间必须 fail-open——文档已如此设计（§2.4、§5.8），
  本次给出实测依据，建议在文档中把该空窗量化写出。

### 3.4 数值一致性（EP 不影响结果）

同一模型在两套 EP 上的输出指纹完全一致：

| 模型 | CPU checksum | DirectML checksum | 相对差异 |
|---|---|---|---|
| `yolo-proxy-640` | 0.000000 | 0.000000 | 0 |
| `ocr-det-proxy-960` | 0.000000 | 0.000000 | 0 |
| `ocr-rec-proxy` | 0.000000 | 0.000000 | 0 |
| `sem-proxy-int64` | 0.002063 | 0.002063 | 2.34e-7 |

（代理模型权重为随机数，checksum 只用于两套 EP 的**互相比对**，无业务含义；FP32 累加顺序差异可忽略。）

---

### 3.5 真实模型复测（BGE 与 RapidOCR 官方权重）

模型来源：`Xenova/bge-small-zh-v1.5`（hf-mirror）、`RapidAI/RapidOCR`（modelscope，PP-OCRv4 mobile 三件套）。

**语义模型（bge-small-zh-v1.5）**：

| 精度 | 体积 | seq=32 CPU p50 | seq=32 DML p50 | seq=128 CPU p50 | seq=128 DML p50 |
|---|---|---|---|---|---|
| fp32 | 94.85 MB | 5.31ms | 2.03ms | — | — |
| **int8** | **24.01 MB** | **2.80ms** | 4.04ms | 9.10ms | 5.08ms |

- 体积压缩 3.95×（FR-SEM-05「常驻约 25MB」实测达标）；CPU 上 int8 比 fp32 快 1.9×。
- **sem 模块默认 CPU 得到真实模型确认**：短草稿（≤32 token，典型场景）CPU 更快；
  长草稿（≥128 token）GPU 反超。两种都在 FR-SEM-05 预算（≤150ms）内余量 16×以上。
- 输出为 `last_hidden_state`，句向量需 mean-pooling + L2 归一化（BGE 官方做法）——
  这一步在 §5.7 的 Embedder 设计中要写明。

**OCR（PP-OCRv4 mobile，release，intra-op=2）**：

| 模型 | 输入 | CPU p50 | DML p50 | 说明 |
|---|---|---|---|---|
| det | 1×3×960×960 | **211.04ms** | **25.75ms**（8.2×） | 全流水线最重的一段，**必须 GPU 优先** |
| rec | 1×3×48×320 | 14.92ms | 6.41ms（2.3×） | 快环按行调用；3 行文本 CPU 需 45ms，仍在预算内 |
| cls | 1×3×48×192 | **0.99ms** | 2.21ms | **CPU 更快**，按模块设备策略归 CPU |

- 三件套合计 16.2MB（文档估的 ~15MB 成立）。
- **修正 §3.3 的结论**：代理模型低估了真实模型的会话就绪成本。真实 det 的 DML
  首次推理（含 EP 图编译）为 **1168ms**（代理模型只测到 10ms）；rec/cls 也在 350~520ms。
  即「挂起 → 卸载 → 唤醒重载」到**可服务**的时长（init + 首跑）：det 约 1.5s（DML）。
  - fail-open 设计完全覆盖该窗口（宁漏勿阻），不构成缺陷；
  - 若 M5 实测挂起/唤醒频繁且漏拦率不可接受，缓解手段是 `ort::compiler::ModelCompiler`
    预编译 EP 图（api-22），或挂起时仅卸载最重的 det 而保留 rec/cls/sem。

## 4. 对设计文档的修订建议

### 4.1 设备策略必须按模块，而非全局开关（建议新增 ADR）

现状：文档只有 `device.prefer_gpu`（全局）。

实测：极小模型 CPU 快 3.7 倍、大模型 GPU 快 10～25 倍。原因是最小模型的耗时由 EP 派发与内存拷贝主导，
计算量不足以摊薄 GPU 开销。

建议：保留 `device.prefer_gpu` 作为**用户可见的全局开关**（NFR-05 要求用户可强制 CPU），
内部增加**按模块的设备策略**（模块可声明 `DevicePref`，缺省按模块特性给默认值）：

| 模块 | 建议默认 | 依据 |
|---|---|---|
| layout（YOLO） | GPU | 10.56 GFLOPs，GPU 快 10.7× |
| ocr det | GPU | 15.26 GFLOPs，GPU 快 9.8× |
| ocr rec | GPU | 7.39 GFLOPs，GPU 快 24.5× |
| sem（int8 句向量） | **CPU 优先** | 短文本算量小；且 GPU 会额外增加一次会话常驻显存 |
| 未来任何 < 0.2 GFLOPs 的新模块 | CPU 优先 | 同上 |

实现上 `PipelineContext.device` 已存在（M2 已落地），只需让 guard 按模块解析。

### 4.2 首次 GPU 就绪空窗（0.35～1.06s）应写进文档

文档 §5.8 只说「重载完成前 fail-open」。建议补一句量化：**进程启动后首个 GPU 会话建立需 0.35～1.06s，
该窗口内所有发送键按 fail-open 放行**（与「宁漏勿阻」一致，不是缺陷）。

### 4.3 ⚠️ §5.7 L2 的「模板相似度」判别力不足（负面发现，必须修订）

用 20 条带标注中文样本（10 安全 / 10 危险）实测 `sim(draft, formal 模板)`：

| 打分法 | 最优准确率 | 结论 |
|---|---|---|
| A 与场景模板余弦相似度（现行 §5.7 设计） | **55%**（≈随机） | 分数挤在 0.38~0.57，两类完全交织 |
| B 与危险样例的最大相似度（BGE 查询前缀） | 50% | 同样无效 |
| C 危险−安全样例差值 | 50% | 同样无效 |
| **线性探针（逻辑回归，LOO 交叉验证）** | **80%**（fp32 与 int8 相同） | **嵌入里有信息，模板余弦取不出来** |

自检排除实现问题：sim(同句)=1.000、流水线正确。结论：

1. bge-small-zh 的嵌入空间**线性可分**地含有「语域/抱怨」信息，但**固定模板的余弦方向不对**；
2. **int8 与 fp32 探针准确率相同** → ADR-10（int8）维持成立；
3. 20 条样本的 LOO 置信区间宽（约 ±18%），80% 是方向性结论，需要 ≥200 条标注集复验。

**修订建议**（需评审拍板）：§5.7 L2 改为「**嵌入 + 预训练线性头**」——

- 线性头 = 512 维权重 + 偏置（2KB），随模型一起发布，按场景画像（formal/casual）各一个；
- 模板相似度降级为零配置的兜底模式（只提示不拦截，或直接移除）；
- 前置工作：产出 ≥200 条标注样本（可与 dc_uitag 的 61 张截图标注并行做），训练脚本放
  `tools/training/`，输出线性头权重到 `models/sem-*/model.toml` 相邻文件；
- 用户「场景可配置」的能力从「改模板文本」变为「导入自训练头」（需要提供训练入口，
  或接受预置头 + 阈值可调）。

### 4.4 DirectML 要求「串行会话执行」

`rapidocr-core` 文档明确：DirectML 需要 DirectX 12 设备**且会话串行执行**。
这与我们的单飞调度（§2.3「同一时刻最多一条流水线在跑」）天然一致，但需要在文档中显式写出约束，
避免将来有人为了「快」而并行跑多个 GPU 会话。

---

## 5. 选型结论

### 5.1 推理运行时：`ort 2.0.0-rc.13`（**采用**）

- 已实测跑通 CPU/DirectML 双端，DLL 自动就位，回退链有效。
- 注意：2.0.0-rc.13 是 rc 版本，需在 M8 固化版本并在 CI 记录 hash（禁 auto-update）。

### 5.2 YOLO 推理 + 后处理：`ultralytics-inference 0.0.44`（**采用并验证**）／自研兜底

- 优点：直接给 letterbox + NMS + 多任务头解码，正是 §5.5 要评估的「现成 crate」。
- 风险：0.0.x 早期版本；需验证**自定义类别数与自定义模型**（我们的 layout 模型是 4 类自训练）。
- 决策规则：若 M4 验证时不支持自定义 4 类模型或无法控制输出，则改为自研后处理
  （letterbox + 解码 + NMS 约 200 行，§5.5 已把它列为 fallback）。

### 5.3 文字识别：`rapidocr-core 0.2.2`（**采用**）

这只 crate 与本项目需求高度拟合，逐条对应：

| 我们的需求 | rapidocr-core 现状 |
|---|---|
| RapidOCR / PaddleOCR ONNX 三件套（ADR-04） | `det → optional cls → rec` 流水线 |
| §2.3 ONNX intra-op 线程受限 | `InferenceOptions` 限制 intra/inter-op 线程与并行度 |
| §4 `CancellationToken` 协作式取消 | `OcrCancellationToken`（可取消进行中的 ONNX run） |
| ADR-05 DirectML | Windows 专属 `directml` 特性 + `ExecutionProvider::DirectMl` |
| 模型可配置/可替换（NFR-08） | `model_set_by_name` / 显式模型路径；模型不打包，可自备 |
| DirectML 串行约束 | 文档已明示「serial session execution」 |

### 5.4 其它候选（**否决**，记录理由）

| 候选 | 结论 | 理由 |
|---|---|---|
| `rapidocr-rs` | 不存在 | crates.io 无此包 |
| `rten 0.26`（纯 Rust 运行时） | 否决 | 无 GPU EP，违反 NFR-05；且需另建一套推理路径 |
| `usls 0.2.0-alpha.3` | 否决 | 仍为 alpha；自带大量模型与 vision 依赖，与「模块化、只带所需」相悖 |
| `rusto-rs` / `paddle-ocr-rs` | 暂缓 | 与 `rapidocr-core` 功能重叠，保留为备选 |

**关键验证**：`cargo tree -p m3-inference -i ort` 显示三者（我们的代码、`rapidocr-core`、
`ultralytics-inference`）依赖的 **都是同一个 `ort v2.0.0-rc.13`**，即只链接一份 ONNX Runtime，
不存在双运行时/DLL 冲突风险。三者均已在本机构建通过（含 `imageproc`、`fast_image_resize` 等传递依赖）。

---

## 6. 未完成项与下一步

| # | 项 | 状态 | 计划 |
|---|---|---|---|
| 1 | ~~int8 vs fp32 准确率/耗时对照~~ | ✅ 已完成（§3.5、§4.3） | 结论：体积/耗时达标，模板相似度判别力不足，需线性头 + ≥200 条样本复验 |
| 2 | **黄金图输出**（M3 DoD 之一） | 待办 | 依赖 #3 的标注数据训练出自训练权重（M4 前置） |
| 3 | 61 张截图标注 | 待办 | ⚠️ 排期问题：文档 §10 并行说明建议「M7 紧随 M3」，否则 M4 无训练数据 → 建议 **M3 收尾后立刻插入 M7** |
| 4 | ~~OCR 真实模型跑通~~ | ✅ 已完成（§3.5） | det/rec/cls 三件套双端耗时已实测 |
| 5 | ~~OCR 真实模型的 DirectML 兼容性~~ | ✅ 已完成（§3.5） | det DML 25.75ms / rec 6.41ms 均正常加速（8.2× / 2.3×），未观察到 LSTM/CTC 算子回退 CPU 子图导致的劣化 |

### 复现命令

```bash
# 0) 生成代理模型（需 Python + onnx 包）
python tools/training/make_bench_models.py spikes/m3-inference/models

# 1) ORT / EP 可用性
cargo run --release -p m3-inference -- info

# 2) 双端耗时（可加 --sessions 2 看同进程重建成本；语义模型用 --seq 指定 token 长度）
cargo run --release -p m3-inference -- bench \
  --model spikes/m3-inference/models/yolo-proxy-640.onnx \
  --shape 1,3,640,640 --runs 20 --ep both --sessions 2 --threads 4

# 3) 模型签名
cargo run -p m3-inference -- sig --model <model.onnx>

# 4) 语义模型 fp32 vs int8 判别力对照（真实权重，模型放 spikes/m3-inference/models/bge/）
#    Windows 默认 GBK 控制台会打印失败，必须加 UTF-8 模式
PYTHONIOENCODING=utf-8 python tools/training/calibrate_sem.py spikes/m3-inference/models/bge
```

（`spikes/**/models/` 已在 `.gitignore` 中，代理模型与真实权重均不入库。）

---

## 7. 需要决策的事项

> **2026-09-13 已全部拍板**，落档见设计文档 §12.1 第 6~8 条、ADR-14/ADR-15、§5.7/§5.8/§10 修订。

1. ~~**设备策略**：是否接受 §4.1 的「按模块设备策略」修订~~ → **已接受**（ADR-15：sem/ocr-cls 默认 CPU，layout/ocr-det/ocr-rec 默认 GPU）。
2. ~~**排期**：是否同意把 M7（dc_uitag）提到 M4 之前~~ → **已同意**（M7 提前为 M3.5，紧随 M3）。
3. ~~**§5.7 L2 修订**：是否接受 §4.3 的「嵌入 + 预训练线性头」方案~~ → **已接受**（ADR-14；模板相似度降级为兜底，前置 ≥200 条语义标注样本）。
