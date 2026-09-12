"""危信 Python 计算面（M0 尖峰阶段）。

当前职责：接收 Rust 尖峰导出的合成 ROI 图像，执行 OCR，
用于测量 M0-C 的标题精确匹配率与草稿字符错误率（CER）。

红线（docs/合规与风险说明.md）：
- 不联网、不落盘任何文本或图像；
- 不枚举窗口、不捕获屏幕、不持有密钥；
- 输入只由 Rust 控制面通过文件参数显式指定。
"""

from __future__ import annotations

import sys

from .ocr_eval import main as _ocr_eval_main
from .ocr_serve import serve as _ocr_serve


def main(argv: list[str]) -> int:
    if len(argv) >= 3 and argv[1] == "ocr-eval":
        return _ocr_eval_main(argv)
    if len(argv) >= 2 and argv[1] == "ocr-serve":
        return _ocr_serve()
    print(
        "用法: python -m dc_worker <ocr-eval <manifest.json> | ocr-serve>",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
