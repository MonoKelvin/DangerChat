# models/ — ONNX 模型仓库

ModelStore（dc-core）按「一目录一模型」扫描：`models/<name>/{model.toml, 权重, LICENSE, 辅助文件}`。
**权重与许可证均已入库**，克隆即可用，无需手动下载。

## 模型清单

| 目录 | 用途 | 权重 | 许可证 |
|---|---|---|---|
| `ocr-det-ppocrv4` | OCR 文本检测 DBNet | `ch_PP-OCRv4_det_mobile.onnx` | Apache-2.0 |
| `ocr-cls-ppocrv20` | OCR 文本行方向分类 | `ch_ppocr_mobile_v2.0_cls_mobile.onnx` | Apache-2.0 |
| `ocr-rec-ppocrv4` | OCR 文本识别 CRNN + 字典 | `ch_PP-OCRv4_rec_mobile.onnx` + `ppocr_keys_v1.txt` | Apache-2.0 |
| `bge` | 语义向量（int8 量化） | `model_quantized.onnx` + `tokenizer.json` | MIT |
| `dc-layout-wechat` | 微信界面四类区域检测 | `yolo11n-dc-layout-wechat-v1.onnx` | 本项目自训练 |

## 来源

- **OCR 三件套**：[RapidAI/RapidOCR](https://www.modelscope.cn/models/RapidAI/RapidOCR) v3.9.0 的 PP-OCRv4 mobile 系列（Apache-2.0）；rec 字典用 PaddleOCR 官方 `ppocr_keys_v1.txt`（v2~v4 中文 rec 通用，6623 字符）。
- **BGE**：[BAAI/bge-small-zh-v1.5](https://huggingface.co/BAAI/bge-small-zh-v1.5)（MIT）的 [Xenova ONNX int8 量化版](https://huggingface.co/Xenova/bge-small-zh-v1.5)。
- **dc-layout-wechat**：`tools/training/train_layout.py` 产出（YOLO11n 微调），重训新版本用 `--ver v2` 递增。

## 注意

- 各模型目录内的 `LICENSE` 为上游许可证原文，改动权重时保留。
- 训练用的原始权重（`yolo11n.pt`、`weights/`、`best.pt`）仍不入库。
- 需要重新下载上游权重时（如换版本）：网络受限环境注意用大写 `HTTP_PROXY/HTTPS_PROXY`。
