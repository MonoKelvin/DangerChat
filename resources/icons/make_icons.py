# -*- coding: utf-8 -*-
"""图标生成脚本：源图 → 全幅拉伸的各尺寸图标（主程序 + uitag 两套）。

用法：python make_icons.py（在 resources/icons/ 下运行）
- 内容裁剪到真实 bbox 后拉伸满幅 512×512（左右/四周无间隙）
- 缩放在**线性光空间**进行（sRGB 直接缩放会让彩色底与浅色图案混出发灰的半调，
  这是小尺寸图标发糊的主要根因）
- ≤128px 的小图加 unsharp mask 锐化补偿，恢复被缩小压平的边缘对比
- 主程序：app-logo.png → 本目录 app-icon-*.png + src-tauri/icons/（ico/png/logo-32）
  + apps/main/public/app-icon.png（前端状态区用）
- uitag：apps/uitag/icons/app-icon.png → apps/uitag/src-tauri/icons/icon.ico
"""
import shutil
from pathlib import Path

import numpy as np
from PIL import Image
from PIL.ImageFilter import UnsharpMask

HERE = Path(__file__).parent
ROOT = HERE.parent.parent


def srgb_to_linear(arr: np.ndarray) -> np.ndarray:
    """sRGB 0..255 → 线性光 0..1（分段函数，标准 sRGB 传递函数）。"""
    c = arr.astype(np.float64) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(arr: np.ndarray) -> np.ndarray:
    """线性光 0..1 → sRGB 0..255。"""
    c = np.clip(arr, 0.0, 1.0)
    s = np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1.0 / 2.4) - 0.055)
    return np.round(s * 255.0).astype(np.uint8)


def resize_linear(img: Image.Image, size: int) -> Image.Image:
    """在线性光空间缩放：预乘 alpha 后缩放，避免透明边缘混入颜色。

    PIL 的 RGBA 不支持 float，把 4 个通道拆成单通道 float32 图逐个缩放。
    """
    a = np.array(img).astype(np.float64)
    rgb = srgb_to_linear(a[..., :3]) * (a[..., 3:4] / 255.0)  # 预乘 alpha
    alpha = a[..., 3] / 255.0
    channels = [rgb[..., i] for i in range(3)] + [alpha]

    resized = [
        np.array(Image.fromarray(ch.astype(np.float32), mode="F").resize((size, size), Image.LANCZOS))
        for ch in channels
    ]

    out_alpha = np.clip(resized[3], 0.0, 1.0)
    # 反预乘：alpha≈0 处 RGB 无意义，置 0 防 NaN
    safe_alpha = np.where(out_alpha < 1e-6, 1.0, out_alpha)
    rgb_lin = np.stack([resized[i] / safe_alpha for i in range(3)], axis=2)
    out_rgb = linear_to_srgb(rgb_lin)
    out_a = np.round(out_alpha * 255.0).astype(np.uint8)
    return Image.fromarray(np.dstack([out_rgb, out_a]).astype(np.uint8), mode="RGBA")


def unsharp(img: Image.Image, size: int) -> Image.Image:
    """小尺寸锐化：unsharp mask 恢复被缩小压平的边缘对比，尺寸越小补偿越强。"""
    if size > 128:
        return img
    strength = {128: 0.25, 64: 0.4, 48: 0.5, 32: 0.6, 16: 0.7}.get(size, 0.5)
    radius = 1.0 if size >= 48 else 0.8
    return img.filter(
        UnsharpMask(radius=radius, percent=int(strength * 100), threshold=1)
    )


def build_frames(src_path: Path):
    """源图 → 裁 bbox → 512 满幅 + 各尺寸锐化帧。"""
    src = Image.open(src_path).convert("RGBA")
    a = np.array(src)
    ys, xs = np.where(a[..., 3] > 10)
    crop = src.crop((int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1))
    print(f"[{src_path.parent.name}/{src_path.name}] bbox {crop.size} → 拉伸 512×512（线性光）")

    full = resize_linear(crop, 512)
    sizes = (256, 128, 64, 48, 32, 24, 16)
    frames = {s: unsharp(resize_linear(crop, s), s) for s in sizes}
    return full, frames


def save_ico(frames, ico_path: Path):
    """写多尺寸 ICO：append_images 提供精确尺寸的锐化帧；基图用 256 帧。"""
    frames[256].save(
        ico_path,
        format="ICO",
        append_images=[frames[s] for s in (128, 64, 48, 32, 24, 16)],
    )


# ── 主程序 ──
full, frames = build_frames(HERE / "app-logo.png")
full.save(HERE / "app-icon-512.png")
for s in (256, 128, 64, 48, 32, 16):
    frames[s].save(HERE / f"app-icon-{s}.png")

tauri_icons = ROOT / "src-tauri" / "icons"
save_ico(frames, tauri_icons / "icon.ico")
shutil.copy(HERE / "app-icon-512.png", tauri_icons / "icon.png")
shutil.copy(HERE / "app-icon-32.png", tauri_icons / "logo-32.png")

public_dir = ROOT / "apps" / "main" / "public"
public_dir.mkdir(exist_ok=True)
shutil.copy(HERE / "app-icon-256.png", public_dir / "app-icon.png")
print("done:", *(p.name for p in sorted(HERE.glob("app-icon-*.png"))))

# ── uitag ──
_, uitag_frames = build_frames(HERE / "uitag-icon.png")
save_ico(uitag_frames, ROOT / "apps" / "uitag" / "src-tauri" / "icons" / "icon.ico")
print("done: uitag-icon.png")
