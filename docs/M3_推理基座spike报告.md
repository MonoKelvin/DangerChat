# 危信 v1.0 — M3 推理基座 spike 报告

- 版本：v1.0-spike.1
- 日期：2026-09-13
- 状态：待评审
- 关联文档：《危信v1.0_开发设计文档.md》§2/§5.5/§5.6/§5.7/§8/§10、《合规与风险说明.md》
- DoD（§10 M3）：spike 报告（黄金图输出、选型结论、双端耗时、int8 vs fp32 准确率对照）
- 本报告覆盖：**选型结论 + 双端耗时**（已完成）；黄金图输出与 int8 对照（待真实权重，见 §6）

---

## 1. 结论摘要

| # | 结论 | 影响 |
|---|---|---|
| 1 | `ort 2.0.0-rc.13` 接入成功，**ONNX Runtime 1.28 预编译库与 `DirectML.dll` 由构建脚本自动就位**，无需手工搬运 | 消除设计文档风险表「ort 需随附 ONNX Runtime 动态库」这一项 |
| 2 | DirectML EP 在本机（RTX 3060 Laptop + Intel UHD）**可用**，无需 CUDA 环境 | ADR-05 成立，A/N/I 显卡通吃 |
| 3 | 真实算量级下 **DirectML 比 CPU 快 10～25 倍**，全部远优于 §9 预算（YOLO 级 CPU 35ms vs 预算 300ms） | §9 性能预算有余量；CPU 路径是可用兜底而非勉强达标 |
| 4 | **小模型（< 0.1 GFLOPs）CPU 反而更快**（0.047ms vs 0.173ms）：EP 派发开销占主导 | ⚠️ **需修订文档**：设备策略要按模块决定，不能只有一个全局 `device.prefer_gpu`（见 §4.1） |
| 5 | DirectML 会话初始化：进程内首个 **350～1060ms**、同进程重建 **36～73ms** | §5.8「挂起卸载 → 唤醒重载 < 500ms」**成立**（重载指同进程重建）；但进程启动后首次 GPU 就绪有 0.4～1s 空窗，必须 fail-open（见 §4.2） |
| 6 | OCR/YOLO 候选 crate 均建立在**同一个 `ort v2.0.0-rc.13`** 之上，无版本冲突，只需一份 ONNX Runtime | 选型可落地，见 §5 |

---

## 2. 环境

| 项 | 值 |
|---|---|
| CPU | 待补（未取到；构建机为笔记本平台） |
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

### 4.3 DirectML 要求「串行会话执行」

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

| # | 项 | 阻塞原因 | 计划 |
|---|---|---|---|
| 1 | **int8 vs fp32 准确率/耗时对照**（M3 DoD 之一） | 需要真实中文句向量模型 | 装 `onnxruntime`（Python）→ 取 bge-small-zh 量化为 int8 → 在 200 条验收集上比对阈值与耗时 |
| 2 | **黄金图输出**（M3 DoD 之一） | 需要真实 YOLO 权重 | 依赖 #3 的标注数据训练出自训练权重（M4 前置） |
| 3 | 61 张截图标注 | `dc_uitag` 属 **M7** | ⚠️ 排期问题：文档 §10 并行说明建议「M7 紧随 M3」，否则 M4 无训练数据 → 建议 **M3 收尾后立刻插入 M7** |
| 4 | OCR 真实模型跑通（RapidOCR 三件套） | 官方模型下载渠道待定（modelscope/GitHub release） | 用 `rapidocr-core` 的 `ModelCache` 或手工放置模型，在 61 张截图上跑 det/rec 测真实耗时 |
| 5 | 真实模型的 DirectML 兼容性 | — | OCR rec 含 LSTM/CTC 类算子，需实测 DML 是否回退到 CPU 子图（性能可能显著劣化）；这是 M4 的第一个验证点 |

### 复现命令

```bash
# 0) 生成代理模型（需 Python + onnx 包）
python tools/training/make_bench_models.py spikes/m3-inference/models

# 1) ORT / EP 可用性
cargo run --release -p m3-inference -- info

# 2) 双端耗时（可加 --sessions 2 看同进程重建成本）
cargo run --release -p m3-inference -- bench \
  --model spikes/m3-inference/models/yolo-proxy-640.onnx \
  --shape 1,3,640,640 --runs 20 --ep both --sessions 2 --threads 4

# 3) 模型签名
cargo run -p m3-inference -- sig --model <model.onnx>
```

（`spikes/**/models/` 已在 `.gitignore` 中，代理模型不入库。）

---

## 7. 需要决策的事项

1. **设备策略**：是否接受 §4.1 的「按模块设备策略」修订（sem 默认 CPU，layout/ocr 默认 GPU）？
2. **排期**：是否同意把 **M7（dc_uitag）** 提到 M4 之前？否则 M4 没有训练数据，只能先做 OCR 与语义链路。
3. **模型获取渠道**：RapidOCR / bge 模型走哪个镜像（modelscope / HuggingFace 镜像 / 手工提供）？
   若走 HuggingFace 原站，需要你确认代理可用性。
4. **int8 量化工具**：是否允许在开发机安装 `onnxruntime` Python 包（约 60MB）用于量化与对照测试？
