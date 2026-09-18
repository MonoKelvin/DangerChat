"""训练 §5.7 场景线性头（ADR-14）：把标注语料嵌入后拟合逻辑回归，
产出 `weights` + `bias` 写入 resources/models/bge/head-{formal,casual}.json。

背景：head 文件此前只有 templates（模板兜底，只提示不拦截，准确率≈随机）。
本脚本产出真实线性头，激活 L2 拦截（score = sigmoid(w·embed + b)）。

语料：resources/training_set/conversation/<category>/*.jsonl（category 最多一层）。
      行 schema 见同目录 schema.json，核心三字段：
        {"text": 运行时嵌入串, "danger": true/false, "base": "formal"|"casual"}
      danger=true → 标签 1（危险，应拦截）；false → 标签 0（安全）。
      base 决定进入哪个头；缺省回落所属目录名（兼容既有的 formal/ casual/ 基线桶）。

特征 = 四段拼接（对齐 crates/dc-pipeline/src/sem/mod.rs::assemble_features）：
      [0:512]    = embed(草稿塔："之前的对话：{ctx}\n现在要发送：{draft}")
      [512:1024] = embed(对象塔："对象：{target}")          ← 每行 object 字段
      [1024:1536]= 草稿 ⊙ 对象（逐元素积）
      [1536:2048]= 草稿 − 对象
      交互项（积/差）不可省：线性头在纯拼接上**可分离**，对象只贡献与草稿无关的
      常数偏移，表达不出「同一句话 × 不同对象 → 相反判定」。
      实测（2026-09-18，1626 条语料含 85 条配对样本）：含交互项 91.8%/85.8%、
      组内配对方向正确率 casual 94%；纯拼接 90.2%/83.9%、方向 82%；
      更早的单塔 86.9%/82.9%、方向≈0。

嵌入：复用 calibrate_sem.py 的 embed()（mean-pooling + L2 归一化，
      与 Rust crates/dc-pipeline/src/sem/embedder.rs 完全对齐）；
      用 int8 量化模型 model_quantized.onnx（与运行时一致）。

用法：
    python tools/training/train_head.py
    python tools/training/train_head.py --model-dir resources/models/bge --dry-run
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

# 与运行时保持一致（crates/dc-pipeline/src/sem/mod.rs::THRESHOLD_*、
# crates/dc-pipeline/src/verdict.rs::BLOCK_MARGIN）
THRESHOLD_FORMAL = 0.55
THRESHOLD_CASUAL = 0.45
BLOCK_MARGIN = 0.25   # 仅用于打印 Block 线；实际扣分在 verdict.rs::from_score
EMBED_DIM = 512          # 单塔维度（= embedder.rs::EMBED_DIM）
FEATURE_DIM = 4 * EMBED_DIM   # 草稿塔 ⊕ 对象塔 ⊕ 逐元素积 ⊕ 差（含交互项）

# 语料允许的判定基线（= 训练出的两个头）
BASES = ("formal", "casual")

REPO_ROOT = Path(__file__).resolve().parents[2]


def embed(session: ort.InferenceSession, tok: Tokenizer, texts: list[str]) -> np.ndarray:
    """mean-pooling + L2 归一化（BGE 官方；与 embedder.rs 对齐）。"""
    names = {i.name for i in session.get_inputs()}
    out = []
    for text in texts:
        enc = tok.encode(text)
        ids = np.array([enc.ids[:128]], dtype=np.int64)
        mask = np.array([enc.attention_mask[:128]], dtype=np.int64)
        feeds = {
            "input_ids": ids,
            "attention_mask": mask,
            "token_type_ids": np.zeros_like(ids),
        }
        feeds = {k: v for k, v in feeds.items() if k in names}
        hidden = session.run(None, feeds)[0]  # [1, seq, 512]
        m = mask[0][:, None].astype(np.float32)
        vec = (hidden[0] * m).sum(axis=0) / np.clip(m.sum(), 1e-9, None)
        vec = vec / np.linalg.norm(vec)
        out.append(vec.astype(np.float32))
    return np.array(out, dtype=np.float32)


def load_corpus(corpus_dir: Path) -> dict[str, tuple[list[str], list[str], np.ndarray]]:
    """递归读取语料并按 `base` 基线分桶（formal / casual 两个头）。

    - 目录：`corpus_dir/<category>/*.jsonl`，category 最多一层；
    - 路由：每行 `base` 字段；缺省回落所属目录名（兼容 formal/ casual/ 基线桶目录）；
    - 去重/冲突：键为 **(text, 对象塔文本)**。配对样本（同一草稿 × 不同对象、标签相反）
      是刻意设计，必须允许；只有同一 (text, 对象) 出现相反标签才算噪声并报错。

    返回 `{base: (draft_texts, object_texts, labels)}`，两个文本列表一一对应。
    """
    buckets: dict[str, dict[tuple[str, str], int]] = {b: {} for b in BASES}
    stats: list[tuple[str, int]] = []
    for f in sorted(corpus_dir.rglob("*.jsonl")):
        if f.parent == corpus_dir:
            continue
        kept = 0
        for lineno, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            base = row.get("base") or f.parent.name
            if base not in buckets:
                raise SystemExit(f"{f}:{lineno} base 非法：{base!r}（应为 {'|'.join(BASES)}）")
            label = 1 if row["danger"] else 0
            obj = "对象：" + (row.get("object") or "未标记")   # 对象塔输入
            key = (row["text"], obj)
            seen = buckets[base]
            if key in seen:
                if seen[key] != label:
                    raise SystemExit(
                        f"{f}:{lineno} 标签冲突：同一 base 内 (text, 对象) 重复且 danger 相反"
                    )
                continue  # 完全重复 → 跳过
            seen[key] = label
            kept += 1
        stats.append((f"{f.parent.name}/{f.name}", kept))

    out: dict[str, tuple[list[str], list[str], np.ndarray]] = {}
    for base, seen in buckets.items():
        keys = list(seen.keys())
        texts = [k[0] for k in keys]
        objs = [k[1] for k in keys]
        labels = np.array([seen[k] for k in keys], dtype=np.float64)
        out[base] = (texts, objs, labels)
    total = sum(len(t) for t, _, _ in out.values())
    print(f"语料：{len(stats)} 个文件 / {total} 条（去重后）")
    for base in BASES:
        t, _, y = out[base]
        if len(t):
            print(f"  [{base}] {len(t)} 条：危险 {int(y.sum())} / 安全 {int((1 - y).sum())}")
    print()
    return out


def fit_logreg(x: np.ndarray, y: np.ndarray, l2: float = 0.5,
               epochs: int = 2000, lr: float = 0.5) -> tuple[np.ndarray, float]:
    """纯 numpy 逻辑回归（与 calibrate_sem.loo_probe 同内核）。返回 (w, b)。"""
    n, dim = x.shape
    w = np.zeros(dim)
    b = 0.0
    for _ in range(epochs):
        z = x @ w + b
        pr = 1.0 / (1.0 + np.exp(-z))
        w -= lr * (x.T @ (pr - y) / n + l2 * w / n)
        b -= lr * float((pr - y).mean())
    return w, b


def loo_accuracy(x: np.ndarray, y: np.ndarray, l2: float = 0.5) -> float:
    """留一交叉验证准确率（泛化能力估计）。小样本用；大样本改 kfold_report。"""
    n = x.shape[0]
    correct = 0
    for held in range(n):
        idx = [i for i in range(n) if i != held]
        w, b = fit_logreg(x[idx], y[idx], l2=l2, epochs=800)
        pred = float(x[held] @ w + b) >= 0.0
        correct += int(pred == bool(y[held]))
    return correct / n


def kfold_report(x: np.ndarray, y: np.ndarray, threshold: float, k: int = 5,
                 l2: float = 0.5, seed: int = 42) -> dict:
    """k 折交叉验证（O(k)，大语料可用）。在**留出集**上按运行时阈值统计，
    给出诚实的误报/漏判估计——训练集准确率会乐观，这个不会。

    返回 acc / 安全误报率(FP) / 危险漏判率(FN)，均为留出集口径。
    """
    n = x.shape[0]
    rng = np.random.default_rng(seed)
    order = rng.permutation(n)
    folds = np.array_split(order, k)
    correct = fp = fn = n_safe = n_danger = 0
    for i in range(k):
        test = folds[i]
        train = np.concatenate([folds[j] for j in range(k) if j != i])
        w, b = fit_logreg(x[train], y[train], l2=l2, epochs=1500)
        s = 1.0 / (1.0 + np.exp(-(x[test] @ w + b)))
        pred = s >= threshold
        yt = y[test].astype(bool)
        correct += int((pred == yt).sum())
        fp += int((pred & ~yt).sum())
        fn += int((~pred & yt).sum())
        n_safe += int((~yt).sum())
        n_danger += int(yt.sum())
    return {
        "acc": correct / n,
        "fp_rate": fp / max(n_safe, 1),
        "fn_rate": fn / max(n_danger, 1),
    }


def train_accuracy(x: np.ndarray, y: np.ndarray, w: np.ndarray, b: float) -> float:
    pred = (x @ w + b) >= 0.0
    return float((pred == y.astype(bool)).mean())


def suggest_threshold(scores: np.ndarray, y: np.ndarray) -> tuple[float, float]:
    """扫 sigmoid 分数阈值：score>=t 判危险，返回 (最优阈值, 准确率)。"""
    best_t, best_acc = 0.5, 0.0
    for t in np.arange(0.20, 0.85, 0.01):
        acc = float(((scores >= t) == y.astype(bool)).mean())
        if acc > best_acc:
            best_t, best_acc = float(t), acc
    return best_t, best_acc


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", default=str(REPO_ROOT / "resources/models/bge"))
    ap.add_argument("--corpus", default=str(REPO_ROOT / "resources/training_set/conversation"))
    ap.add_argument("--dry-run", action="store_true", help="只训练打印，不写 head 文件")
    ap.add_argument("--no-loo", action="store_true", help="跳过 LOO 交叉验证（语料增大后耗时线性上升）")
    args = ap.parse_args()

    model_dir = Path(args.model_dir)
    corpus_dir = Path(args.corpus)
    tok = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
    weight = model_dir / "model_quantized.onnx"
    sess = ort.InferenceSession(str(weight), providers=["CPUExecutionProvider"])
    print(f"模型：{weight}（int8，与运行时一致）\n")

    corpus = load_corpus(corpus_dir)

    scenes = [
        ("formal", "head-formal.json", THRESHOLD_FORMAL),
        ("casual", "head-casual.json", THRESHOLD_CASUAL),
    ]

    for scene, head_file, threshold in scenes:
        texts, objs, y = corpus[scene]
        if len(texts) == 0:
            print(f"[{scene}] 无语料，跳过")
            continue
        # 特征装配顺序必须与 sem/mod.rs::assemble_features 一致（改动即需同步两者）
        xd = embed(sess, tok, texts)
        xo = embed(sess, tok, objs)
        x = np.concatenate([xd, xo, xd * xo, xd - xo], axis=1)
        assert x.shape[1] == FEATURE_DIM, f"特征维度 {x.shape[1]} != {FEATURE_DIM}"
        w, b = fit_logreg(x, y)
        scores = 1.0 / (1.0 + np.exp(-(x @ w + b)))

        tr_acc = train_accuracy(x, y, w, b)
        best_t, best_acc = suggest_threshold(scores, y)
        block_line = threshold + BLOCK_MARGIN
        acc_at_threshold = float(((scores >= threshold) == y.astype(bool)).mean())
        cv = None if args.no_loo else kfold_report(x, y, threshold)

        print(f"=== {scene}（{len(texts)} 条：危险 {int(y.sum())} / 安全 {int((1-y).sum())}）===")
        print(f"  训练集准确率     : {tr_acc*100:.1f}%")
        if cv is not None:
            print(f"  5折交叉验证(留出): acc {cv['acc']*100:.1f}%  "
                  f"安全误报 {cv['fp_rate']*100:.1f}%  危险漏判 {cv['fn_rate']*100:.1f}%")
        else:
            print("  交叉验证         : 已跳过（--no-loo）")
        print(f"  当前阈值 {threshold:.2f} / Block线 {block_line:.2f}")
        print(f"  该阈值下准确率   : {acc_at_threshold*100:.1f}%")
        print(f"  阈值扫描最优     : t={best_t:.2f} / 准确率 {best_acc*100:.1f}%")
        # 危险/安全两类的分数分布，供判断阈值是否合理
        print(f"  危险类分数 mean={scores[y==1].mean():.3f} min={scores[y==1].min():.3f}")
        print(f"  安全类分数 mean={scores[y==0].mean():.3f} max={scores[y==0].max():.3f}")

        if not args.dry_run:
            head_path = model_dir / head_file
            existing = json.loads(head_path.read_text(encoding="utf-8"))
            # dim=2048（4×512）→ 运行时启用 草稿⊕对象⊕积⊕差（sem/head.rs 接受 512/1024/2048）
            existing["dim"] = FEATURE_DIM
            existing["weights"] = [float(v) for v in w]
            existing["bias"] = float(b)
            # 保留 templates（兜底仍可用）
            head_path.write_text(
                json.dumps(existing, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            print(f"  → 已写入 {head_path}")
        print()

    if args.dry_run:
        print("dry-run：未写 head 文件")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
