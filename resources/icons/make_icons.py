# -*- coding: utf-8 -*-
"""图标生成脚本：app-logo.png → 全幅拉伸的各尺寸图标。

用法：python make_icons.py（在 resources/icons/ 下运行）
- 内容裁剪到真实 bbox 后拉伸满幅 512×512（左右/四周无间隙）
- 输出常用 PNG 尺寸到本目录
- 生成 src-tauri/icons/icon.ico（多尺寸）+ icon.png + logo-32.png（托盘基础图）
- 输出 apps/main/public/app-icon.png（前端状态区用）
"""
import shutil
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).parent
ROOT = HERE.parent.parent

src = Image.open(HERE / "app-logo.png").convert("RGBA")
a = np.array(src)
ys, xs = np.where(a[..., 3] > 10)
crop = src.crop((int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1))
print(f"内容 bbox: {crop.size} → 拉伸 512×512")

full = crop.resize((512, 512), Image.LANCZOS)
full.save(HERE / "app-icon-512.png")

for s in (256, 128, 64, 48, 32, 16):
    full.resize((s, s), Image.LANCZOS).save(HERE / f"app-icon-{s}.png")

tauri_icons = ROOT / "src-tauri" / "icons"
full.save(tauri_icons / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
shutil.copy(HERE / "app-icon-512.png", tauri_icons / "icon.png")
shutil.copy(HERE / "app-icon-32.png", tauri_icons / "logo-32.png")

public_dir = ROOT / "apps" / "main" / "public"
public_dir.mkdir(exist_ok=True)
shutil.copy(HERE / "app-icon-256.png", public_dir / "app-icon.png")
print("done:", *(p.name for p in sorted(HERE.glob("app-icon-*.png"))))
