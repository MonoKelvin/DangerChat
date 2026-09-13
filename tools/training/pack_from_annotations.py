"""从 dc_uitag 的 annotations.json（自动保存现场）直接合成训练集 zip。

等价于在 uitag 界面点「导出训练集」——供训练脚本消费，无需开 GUI。
复用 apps/uitag 的导出契约：images/<stem>.png + labels/<stem>.txt（YOLO 归一化）
+ classes.txt + tags.json。

用法：
    python tools/training/pack_from_annotations.py <annotations.json> <out.zip>
"""

from __future__ import annotations

import json
import sys
import zipfile
from pathlib import Path

CLASSES = ["chat_list", "chat_window", "chat_target", "msg_input"]


def main() -> int:
    src = Path(sys.argv[1] if len(sys.argv) > 1 else r"D:\WEB\Rust\target\debug\annotations.json")
    out = Path(sys.argv[2] if len(sys.argv) > 2 else "uitag-dataset.zip")

    with open(src, encoding="utf-8") as f:
        annos = json.load(f)["annos"]

    labeled = 0
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
        for path, boxes in sorted(annos.items()):
            p = Path(path)
            if not p.is_file():
                print(f"跳过（文件不存在）：{p.name}")
                continue
            zf.write(p, f"images/{p.name}")
            stem = p.stem
            lines = []
            for b in boxes:
                cls = CLASSES.index(b["tag"])
                # 像素 → 归一化中心
                import struct

                with open(p, "rb") as fp:
                    fp.read(16)
                    w, h = struct.unpack(">II", fp.read(8))
                cx = (b["x"] + b["w"] / 2) / w
                cy = (b["y"] + b["h"] / 2) / h
                nw, nh = b["w"] / w, b["h"] / h
                lines.append(f"{cls} {cx:.10f} {cy:.10f} {nw:.10f} {nh:.10f}")
            zf.writestr(f"labels/{stem}.txt", "\n".join(lines))
            if lines:
                labeled += 1
        zf.writestr("classes.txt", "\n".join(CLASSES) + "\n")
        zf.writestr(
            "tags.json",
            json.dumps(
                {
                    "version": 1,
                    "tags": [
                        {"name": "chat_list", "label": "聊天列表", "color": "#4C9AFF", "enabled": True},
                        {"name": "chat_window", "label": "聊天窗口", "color": "#36B37E", "enabled": True},
                        {"name": "chat_target", "label": "聊天对象", "color": "#FFAB00", "enabled": True},
                        {"name": "msg_input", "label": "消息输入框", "color": "#FF5630", "enabled": True},
                    ],
                },
                ensure_ascii=False,
                indent=2,
            ),
        )
    print(f"{out}：{len(annos)} 张图，{labeled} 张已标注")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
