"""M4：layout 区域模型训练（设计文档 §8 离线工具链）。

消费 dc_uitag 导出的 YOLO 数据集 zip（images/ + labels/ + classes.txt + tags.json），
用 ultralytics YOLO11n 迁移学习，产出 models/layout-wechat/{model.toml, *.onnx}。

关键决策（M4 计划拍板）：
  · 增广**禁用**：UI 截图的颜色与几何是语义（深浅主题是特征不是噪声），
    hsv/翻转/mosaic/mixup/旋转全部关死，只留 translate=0.05 / scale=0.1
    容忍标注抖动；小数据集（73 张）防过拟合靠预训练底座 + 早停。
  · classes 顺序 = uitag classes.txt 顺序（chat_list/chat_window/chat_target/msg_input），
    两端由该顺序钉死，model.toml 的 classes 与训练 data.yaml 必须一致。

用法：
    python tools/training/train_layout.py <dataset.zip> [--smoke]
    # --smoke：当前部分标注即可跑，1 epoch 验证链路（产出不进 models/）
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
EXPECTED_CLASSES = ["chat_list", "chat_window", "chat_target", "msg_input"]
EPOCHS = 150
PATIENCE = 20


def extract_dataset(zip_path: Path, workdir: Path) -> Path:
    with zipfile.ZipFile(zip_path) as zf:
        zf.extractall(workdir)
    classes_txt = workdir / "classes.txt"
    if not classes_txt.is_file():
        sys.exit(f"数据集缺 classes.txt：{zip_path}")
    classes = [l.strip() for l in classes_txt.read_text(encoding="utf-8").splitlines() if l.strip()]
    if classes != EXPECTED_CLASSES:
        sys.exit(f"classes.txt 顺序不符：{classes}（期望 {EXPECTED_CLASSES}）")
    return workdir


def split_dataset(root: Path, out: Path, val_ratio: float = 0.2, seed: int = 42) -> None:
    """固定 seed 的 8:2 划分（73 张小集，划分稳定比交叉验证重要）。"""
    import random

    images = sorted((root / "images").glob("*.png"))
    random.Random(seed).shuffle(images)
    n_val = max(1, int(len(images) * val_ratio))
    val, train = images[:n_val], images[n_val:]

    for sub in ("train", "val"):
        (out / "images" / sub).mkdir(parents=True, exist_ok=True)
        (out / "labels" / sub).mkdir(parents=True, exist_ok=True)
    for img in train:
        shutil.copy2(img, out / "images" / "train" / img.name)
        lbl = root / "labels" / f"{img.stem}.txt"
        if lbl.is_file():
            shutil.copy2(lbl, out / "labels" / "train" / lbl.name)
    for img in val:
        shutil.copy2(img, out / "images" / "val" / img.name)
        lbl = root / "labels" / f"{img.stem}.txt"
        if lbl.is_file():
            shutil.copy2(lbl, out / "labels" / "val" / lbl.name)

    (out / "data.yaml").write_text(
        f"path: {out.resolve().as_posix()}\n"
        "train: images/train\n"
        "val: images/val\n"
        f"nc: {len(EXPECTED_CLASSES)}\n"
        f"names: {EXPECTED_CLASSES}\n",
        encoding="utf-8",
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dataset", type=Path, help="dc_uitag 导出的数据集 zip")
    ap.add_argument("--smoke", action="store_true", help="1 epoch 链路验证（不写入 models/）")
    ap.add_argument("--epochs", type=int, default=EPOCHS)
    args = ap.parse_args()

    import torch

    # Windows + torch CPU：默认 8 线程在本机（11800H）多 epoch 训练时段错误
    # （crash 点随机在 batch 边界，非代码路径问题）；限 2 线程训练更稳且小数据集无速度损失
    torch.set_num_threads(2)

    from ultralytics import YOLO

    work = Path(tempfile.mkdtemp(prefix="uitag-train-"))
    ds_root = extract_dataset(args.dataset, work)
    split_dataset(ds_root, work)

    model = YOLO("yolo11n.pt")
    results = model.train(
        data=str(work / "data.yaml"),
        epochs=1 if args.smoke else args.epochs,
        patience=PATIENCE,
        imgsz=640,
        seed=42,
        # batch=4：默认 auto(16) 在低提交内存机器上训练段错误（实测 11800H/16GB
        # + 页面文件 12G 时 batch 边界随机崩溃）；4 对 59 张小数据集足够稳定
        batch=4,
        # —— 增广禁用（UI 截图语义）——
        hsv_h=0.0, hsv_s=0.0, hsv_v=0.0,
        degrees=0.0, translate=0.05, scale=0.1, shear=0.0,
        perspective=0.0, flipud=0.0, fliplr=0.0,
        mosaic=0.0, mixup=0.0, copy_paste=0.0,
        erasing=0.0,
        project=str(work / "runs"),
        name="layout",
    )

    best = Path(results.save_dir) / "weights" / "best.pt"
    if not best.is_file():
        sys.exit(f"训练未产出 best.pt：{results.save_dir}")

    if args.smoke:
        print(f"[smoke] 链路 OK，权重在 {best}（不写入 models/）")
        print(f"[smoke] 指标：{results.results_dict}")
        return 0

    # 导出 ONNX + 组装 models/layout-wechat/
    onnx_path = best.with_suffix(".onnx")
    exported = model.export(format="onnx", opset=12, simplify=True)
    exported_path = Path(exported)

    dest = REPO / "models" / "layout-wechat"
    dest.mkdir(parents=True, exist_ok=True)
    shutil.copy2(exported_path, dest / "layout-wechat.onnx")
    (dest / "model.toml").write_text(
        "kind = \"layout\"\n"
        "file = \"layout-wechat.onnx\"\n"
        "version = \"yolo11n-wechat-73\"\n"
        "input_size = 640\n"
        f"classes = {EXPECTED_CLASSES}\n"
        'note = "dc_uitag 标注 73 张训练；增广禁用（UI 截图语义）"\n',
        encoding="utf-8",
    )
    print(f"已产出 {dest}")
    print(f"指标：{results.results_dict}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
