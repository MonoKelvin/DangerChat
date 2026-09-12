"""保活 OCR 服务（M0-D 尖峰）。

简化行协议，仅为尖峰延迟测量服务；正式的 MessagePack 管道协议属 M1。

输入：每行一个 PNG 路径。
输出：识别文本一行（空白归一化后为空时输出空行）。

stderr 写日志，stdout 只写协议，符合最终 Worker 的约束。
"""

from __future__ import annotations

import sys
from pathlib import Path

from rapidocr_onnxruntime import RapidOCR


def serve() -> int:
    engine = RapidOCR()
    # 预热：首次推理含模型加载，不计入测量。
    try:
        engine(b"\x89PNG\r\n\x1a\n")
    except Exception:
        pass
    print("READY", flush=True)

    for line in sys.stdin:
        path = line.strip()
        if not path:
            continue
        if path == "SHUTDOWN":
            break
        try:
            outcome, _ = engine(Path(path).read_bytes())
            text = "".join(str(row[1]) for row in outcome).strip() if outcome else ""
        except Exception as exc:  # noqa: BLE001 - 协议层必须吞掉异常并回报
            print(f"__ERROR__ {exc}", flush=True)
            continue
        # 识别文本不会包含换行（行已拼接），可以安全按行回传。
        print(text, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(serve())
