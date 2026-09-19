# models/ — ONNX 模型仓库

ModelStore（dc-core）按「一目录一模型」扫描：`models/<name>/{model.toml, 权重, LICENSE, 辅助文件}`。
多数权重已入库，克隆即可用；**唯一例外是 `bge-large/model_quantized.onnx`（311MB，超 GitHub 单文件限制，不入库），需手动下载**——见下方「获取 bge-large 权重」。

## 模型清单

| 目录 | 用途 | 权重 | 许可证 |
|---|---|---|---|
| `ocr-det-ppocrv4` | OCR 文本检测 DBNet | `ch_PP-OCRv4_det_mobile.onnx` | Apache-2.0 |
| `ocr-cls-ppocrv20` | OCR 文本行方向分类 | `ch_ppocr_mobile_v2.0_cls_mobile.onnx` | Apache-2.0 |
| `ocr-rec-ppocrv4` | OCR 文本识别 CRNN + 字典 | `ch_PP-OCRv4_rec_mobile.onnx` + `ppocr_keys_v1.txt` | Apache-2.0 |
| `bge-large` | 语义向量（1024 维，int8 量化） | `model_quantized.onnx`（**手动下载**）+ `tokenizer.json` | MIT |
| `dc-layout-wechat` | 微信界面四类区域检测 | `yolo11n-dc-layout-wechat-v1.onnx` | 本项目自训练 |

## 获取 bge-large 权重（克隆后必做一次）

`model_quantized.onnx` 未入库，缺失时 sem 模块 fail-open（仅 L1 规则，L2 语义判定不生效）。
下载 Xenova 转换版 `model_int8.onnx` 重命名为 `model_quantized.onnx` 放入 `resources/models/bge-large/`：

```bash
# HF 直连；国内用镜像加 HF_ENDPOINT=https://hf-mirror.com
python -c "from huggingface_hub import hf_hub_download; import shutil,os; \
p=hf_hub_download('Xenova/bge-large-zh-v1.5','onnx/model_int8.onnx',local_dir='/tmp/bge'); \
shutil.copy(p,'resources/models/bge-large/model_quantized.onnx')"
```

也可从 `https://hf-mirror.com/Xenova/bge-large-zh-v1.5/resolve/main/onnx/model_int8.onnx` 直接下载后手动重命名。
下载后确认输出维度 1024、三输入 `input_ids/attention_mask/token_type_ids`（与 `head-*.json` 的 4096 维双塔头匹配）。

## 来源

- **OCR 三件套**：[RapidAI/RapidOCR](https://www.modelscope.cn/models/RapidAI/RapidOCR) v3.9.0 的 PP-OCRv4 mobile 系列（Apache-2.0）；rec 字典用 PaddleOCR 官方 `ppocr_keys_v1.txt`（v2~v4 中文 rec 通用，6623 字符）。
- **BGE**：[BAAI/bge-large-zh-v1.5](https://huggingface.co/BAAI/bge-large-zh-v1.5)（MIT）的 [Xenova ONNX int8 量化版](https://huggingface.co/Xenova/bge-large-zh-v1.5)（`model_int8.onnx`，1024 维）。语义判定天花板受编码器可分性限制，large 较 small 明显提升；int8 与 fp32 实测准确率无差异，故取 int8。会话常驻约 350MB，靠挂起态装卸（目标离开前台即卸载）压低运行时内存。
- **dc-layout-wechat**：`tools/training/train_layout.py` 产出（YOLO11n 微调），重训新版本用 `--ver v2` 递增。

## 替换语义模型（可选）

语义判定模型（`kind=sem`）可切换：默认内置 `bge-large`，用户可放入自己的模型后在
**设置 → 高级 → 语义判定 → 语义模型**下拉里选择（重启生效）。放置方式与内置一致：

```
<数据目录>/models/<你的模型名>/
  model.toml          # kind = "sem"，file = 权重文件名
  <权重>.onnx         # 句向量编码器（三输入 i64：input_ids/attention_mask/token_type_ids）
  tokenizer.json      # 分词器
  head-formal.json    # 线性头（dim/weights/bias，见 tools/training/train_head.py）
  head-casual.json
```

维度需与 `crates/dc-pipeline/src/sem/embedder.rs::EMBED_DIM` 一致（当前 1024）。用户层同名目录
覆盖内置层。头文件缺 `weights` → 退化为模板兜底（只提示不拦截）。

## 注意

- 各模型目录内的 `LICENSE` 为上游许可证原文，改动权重时保留。
- 训练用的原始权重（`yolo11n.pt`、`weights/`、`best.pt`）仍不入库。
- 需要重新下载上游权重时（如换版本）：网络受限环境注意用大写 `HTTP_PROXY/HTTPS_PROXY`。
