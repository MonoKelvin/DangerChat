# models/ — ONNX 模型仓库（不入库）

ModelStore（dc-core）按「一目录一模型」扫描：`models/<name>/{model.toml, 权重, 辅助文件}`。

## 当前模型

| 目录 | 用途 | 来源 |
|---|---|---|
| `ocr-det-ppocrv4` | OCR 文本检测 DBNet | [RapidAI/RapidOCR](https://www.modelscope.cn/models/RapidAI/RapidOCR) v3.9.0 `onnx/PP-OCRv4/det/ch_PP-OCRv4_det_mobile.onnx` |
| `ocr-cls-ppocrv20` | OCR 文本行方向分类 | 同上 `onnx/PP-OCRv4/cls/ch_ppocr_mobile_v2.0_cls_mobile.onnx` |
| `ocr-rec-ppocrv4` | OCR 文本识别 CRNN + 字典 | 模型同上 `onnx/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile.onnx`；字典 [PaddleOCR v2.7.0](https://github.com/PaddlePaddle/PaddleOCR/blob/v2.7.0/ppocr/utils/ppocr_keys_v1.txt)（6623 字符） |
| `layout-wechat`（待训练） | 危信四类区域检测 YOLO | `tools/training/train_layout.py` 产出（M4 训练完成后） |

> `.onnx` / 大字典文件已 gitignore；新环境按上表下载放置。`model.toml` 入库。

## 注意

- PP-OCRv4 中文 rec 的字典在 modelscope 上已无 v4 路径，实际用 PaddleOCR 官方 `ppocr_keys_v1.txt`（v2~v4 中文 rec 通用，6623 字符）。
- 下载需要代理时用大写 `HTTP_PROXY/HTTPS_PROXY`。
