# models/ — ONNX 模型仓库

ModelStore（dc-core）按「一目录一模型」扫描：`models/<name>/{model.toml, 权重, LICENSE, 辅助文件}`。
多数权重已入库，克隆即可用；语义模型 `bge-small` 的 ONNX 权重约 24MB，随仓库提供。

## 模型清单

| 目录 | 用途 | 权重 | 许可证 |
|---|---|---|---|
| `ocr-det-ppocrv4` | OCR 文本检测 DBNet | `ch_PP-OCRv4_det_mobile.onnx` | Apache-2.0 |
| `ocr-cls-ppocrv20` | OCR 文本行方向分类 | `ch_ppocr_mobile_v2.0_cls_mobile.onnx` | Apache-2.0 |
| `ocr-rec-ppocrv4` | OCR 文本识别 CRNN + 字典 | `ch_PP-OCRv4_rec_mobile.onnx` + `ppocr_keys_v1.txt` | Apache-2.0 |
| `bge-small` | 语义向量（512 维，int8 量化） | `model_quantized.onnx` + `tokenizer.json` + 双塔线性头 | MIT |
| `dc-layout-wechat` | 微信界面四类区域检测 | `yolo11n-dc-layout-wechat-v1.onnx` | 本项目自训练 |

## 获取 bge-small 权重（克隆后已随仓库提供）

`bge-small/model_quantized.onnx` 为 Xenova 转换版 bge-small-zh-v1.5 的 int8 权重，输出维度 512，包含三输入 `input_ids/attention_mask/token_type_ids`，与 `head-*.json` 的 2048 维双塔头匹配。

## 来源

- **OCR 三件套**：[RapidAI/RapidOCR](https://www.modelscope.cn/models/RapidAI/RapidOCR) v3.9.0 的 PP-OCRv4 mobile 系列（Apache-2.0）；rec 字典用 PaddleOCR 官方 `ppocr_keys_v1.txt`（v2~v4 中文 rec 通用，6623 字符）。
- **BGE**：[BAAI/bge-small-zh-v1.5](https://huggingface.co/BAAI/bge-small-zh-v1.5)（MIT）的 [Xenova ONNX int8 量化版](https://huggingface.co/Xenova/bge-small-zh-v1.5)（512 维）。语义判定使用标记位置读取（jev 风格）+ L2 归一化，双塔交互线性头保留对象场景信息；CPU 短文本推理满足 FR-SEM-05 的延迟与内存预算。
- **dc-layout-wechat**：`tools/training/train_layout.py` 产出（YOLO11n 微调），重训新版本用 `--ver v2` 递增。

## 替换语义模型（可选）

语义判定模型（`kind=sem`）可切换：默认内置 `bge-small`，用户可放入自己的模型后在
**设置 → 高级 → 语义判定 → 语义模型**下拉里选择（重启生效）。放置方式与内置一致：

```
<数据目录>/models/<你的模型名>/
  model.toml          # kind = "sem"，file = 权重文件名
  <权重>.onnx         # 句向量编码器（三输入 i64：input_ids/attention_mask/token_type_ids）
  tokenizer.json      # 分词器
  head-formal.json    # 线性头（dim/weights/bias，见 tools/training/train_head.py）
  head-casual.json
```

维度需与 `crates/dc-pipeline/src/sem/embedder.rs::EMBED_DIM` 一致（当前 512）；双塔线性头维度为 4×512=2048。用户层同名目录
覆盖内置层。头文件缺 `weights` → 退化为模板兜底（只提示不拦截）。

## 注意

- 各模型目录内的 `LICENSE` 为上游许可证原文，改动权重时保留。
- 训练用的原始权重（`yolo11n.pt`、`weights/`、`best.pt`）仍不入库。
- 需要重新下载上游权重时（如换版本）：网络受限环境注意用大写 `HTTP_PROXY/HTTPS_PROXY`。
