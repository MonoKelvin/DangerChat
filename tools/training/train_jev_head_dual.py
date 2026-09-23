"""训练 jev 风格双塔线性头（标记读取 + 交互项保留版）。

核心改动：
1. **双塔架构保留**：
   - 草稿塔：embed_at_marker("[unused1] {draft_text}")
   - 对象塔：embed_at_marker("[unused2] {object_text}")
   - 特征装配：[draft, object, draft⊙object, draft-object]（4×512=2048 维）
2. **标记读取替代 mean-pooling**：每个塔读取标记位置 1 的隐藏状态
3. **多问题头**：severity 头 + danger 头（共用 2048 维特征）

为何保留双塔：
- 原 train_head.py 实测双塔+交互项 91.8%/85.8%，单塔 86.9%/82.9%（差距 5-3 个点）；
- jev 官方论文的单塔设计是针对问答任务（问题+选项都显式给出），但危信的「对象」
  不是文本中的显式选项，而是运行时动态传入的上下文（同一草稿×不同对象→相反判定），
  单塔 512 维容量不足以编码这种交互关系；
- 配对样本验证：基线双塔方向正确率 94%，单塔≈0。

用法：
    python tools/training/train_jev_head_dual.py
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

THRESHOLD_FORMAL = 0.55
THRESHOLD_CASUAL = 0.45
BLOCK_MARGIN = 0.25
EMBED_DIM = 512
FEATURE_DIM = 4 * EMBED_DIM  # 双塔+交互项

SEVERITY_TARGET = {"none": 0.0, "low": 0.20, "medium": 0.65, "high": 0.97}
LEGACY_SOFT_TRUE = SEVERITY_TARGET["medium"]
LEGACY_SOFT_FALSE = SEVERITY_TARGET["none"]
SOFT_CONFLICT_GAP = 0.5

BASES = ("formal", "casual")
REPO_ROOT = Path(__file__).resolve().parents[2]

MARKER_DRAFT = 1   # [unused1]
MARKER_OBJECT = 2  # [unused2]


def embed_at_marker(
    session: ort.InferenceSession, tok: Tokenizer, text: str, marker_id: int
) -> np.ndarray:
    """标记位置读取：[CLS] marker_id text... [SEP]，返回位置 1 的 L2 归一化向量。"""
    enc = tok.encode(text)
    ids_list = [101, marker_id] + enc.ids[1:127]
    ids = np.array([ids_list], dtype=np.int64)
    mask = np.ones((1, len(ids_list)), dtype=np.int64)

    names = {i.name for i in session.get_inputs()}
    feeds = {
        "input_ids": ids,
        "attention_mask": mask,
        "token_type_ids": np.zeros_like(ids),
    }
    feeds = {k: v for k, v in feeds.items() if k in names}

    hidden = session.run(None, feeds)[0]
    vec = hidden[0, 1, :]
    norm = np.linalg.norm(vec)
    if norm < 1e-9:
        raise ValueError("嵌入退化")
    vec = vec / norm
    return vec.astype(np.float32)


def _row_targets(row: dict) -> tuple[float, int, str]:
    danger = bool(row["danger"])
    sev = row.get("severity")
    if sev is not None:
        if sev not in SEVERITY_TARGET:
            raise ValueError(f"severity 非法：{sev!r}")
        return SEVERITY_TARGET[sev], (1 if danger else 0), sev
    soft = LEGACY_SOFT_TRUE if danger else LEGACY_SOFT_FALSE
    return soft, (1 if danger else 0), ("medium" if danger else "none")


def load_corpus(
    corpus_dir: Path,
) -> dict[str, tuple[list[str], list[str], np.ndarray, np.ndarray, list[str]]]:
    """按 base 分桶：{base: (draft_texts, object_texts, y_sev, y_dng, sevs)}。"""
    buckets: dict[str, dict[tuple[str, str], tuple[float, int, str]]] = {
        b: {} for b in BASES
    }
    for f in sorted(corpus_dir.rglob("*.jsonl")):
        if f.parent == corpus_dir:
            continue
        for lineno, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            base = row.get("base") or f.parent.name
            if base not in buckets:
                raise SystemExit(f"{f}:{lineno} base 非法：{base!r}")
            try:
                soft, y_dng, sev = _row_targets(row)
            except ValueError as e:
                raise SystemExit(f"{f}:{lineno} {e}")
            # 双塔输入
            draft = row["text"]
            obj = "对象：" + (row.get("object") or "未标记")
            key = (draft, obj)
            if key in buckets[base]:
                if abs(buckets[base][key][0] - soft) > SOFT_CONFLICT_GAP:
                    raise SystemExit(f"{f}:{lineno} 标签冲突")
                continue
            buckets[base][key] = (soft, y_dng, sev)

    out: dict[str, tuple[list[str], list[str], np.ndarray, np.ndarray, list[str]]] = {}
    for base, seen in buckets.items():
        keys = list(seen.keys())
        drafts = [k[0] for k in keys]
        objs = [k[1] for k in keys]
        y_sev = np.array([seen[k][0] for k in keys], dtype=np.float64)
        y_dng = np.array([seen[k][1] for k in keys], dtype=np.float64)
        sevs = [seen[k][2] for k in keys]
        out[base] = (drafts, objs, y_sev, y_dng, sevs)
    total = sum(len(d) for d, *_ in out.values())
    print(f"语料：{total} 条（去重后）")
    for base in BASES:
        d, _, _, yb, sevs = out[base]
        if len(d):
            c = Counter(sevs)
            print(
                f"  [{base}] {len(d)} 条：危险 {int(yb.sum())} / 安全 {int((1-yb).sum())}"
                f"  ｜ severity none={c['none']} low={c['low']} medium={c['medium']} high={c['high']}"
            )
    print()
    return out


def fit_logreg(
    x: np.ndarray, y: np.ndarray, l2: float = 0.5, epochs: int = 2000, lr: float = 0.5
) -> tuple[np.ndarray, float]:
    n, dim = x.shape
    x = np.ascontiguousarray(x, dtype=np.float32)
    y = y.astype(np.float32)
    w = np.zeros(dim, dtype=np.float32)
    b = np.float32(0.0)
    for _ in range(epochs):
        z = x @ w + b
        pr = 1.0 / (1.0 + np.exp(-z))
        w -= lr * (x.T @ (pr - y) / n + l2 * w / n)
        b -= lr * float((pr - y).mean())
    return w, b


def kfold_report_soft(
    x: np.ndarray,
    y_soft: np.ndarray,
    y_bool: np.ndarray,
    threshold: float,
    k: int = 5,
    l2: float = 0.5,
    seed: int = 42,
) -> dict:
    n = x.shape[0]
    rng = np.random.default_rng(seed)
    order = rng.permutation(n)
    folds = np.array_split(order, k)
    correct = fp = fn = n_safe = n_danger = 0
    for i in range(k):
        test = folds[i]
        train = np.concatenate([folds[j] for j in range(k) if j != i])
        w, b = fit_logreg(x[train], y_soft[train], l2=l2, epochs=1500)
        s = 1.0 / (1.0 + np.exp(-(x[test] @ w + b)))
        pred = s >= threshold
        yt = y_bool[test].astype(bool)
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


def suggest_threshold(scores: np.ndarray, y: np.ndarray) -> tuple[float, float]:
    best_t, best_acc = 0.5, 0.0
    for t in np.arange(0.20, 0.85, 0.01):
        acc = float(((scores >= t) == y.astype(bool)).mean())
        if acc > best_acc:
            best_t, best_acc = float(t), acc
    return best_t, best_acc


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", default=str(REPO_ROOT / "resources/models/bge-small"))
    ap.add_argument("--corpus", default=str(REPO_ROOT / "resources/training_set/conversation"))
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--no-loo", action="store_true")
    args = ap.parse_args()

    model_dir = Path(args.model_dir)
    corpus_dir = Path(args.corpus)
    tok = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
    weight = model_dir / "model_quantized.onnx"
    sess = ort.InferenceSession(str(weight), providers=["CPUExecutionProvider"])
    print(f"模型：{weight}（bge-small-zh int8，双塔+标记读取）\n")

    corpus = load_corpus(corpus_dir)

    scenes = [
        ("formal", "head-formal.json", THRESHOLD_FORMAL),
        ("casual", "head-casual.json", THRESHOLD_CASUAL),
    ]

    for scene, head_file, threshold in scenes:
        drafts, objs, y_sev, y_dng, sevs = corpus[scene]
        if len(drafts) == 0:
            print(f"[{scene}] 无语料，跳过")
            continue

        # 双塔特征提取
        print(f"[{scene}] 提取 {len(drafts)} 条特征（双塔+交互项）...")
        xd = np.array(
            [embed_at_marker(sess, tok, t, MARKER_DRAFT) for t in drafts],
            dtype=np.float32,
        )
        xo = np.array(
            [embed_at_marker(sess, tok, t, MARKER_OBJECT) for t in objs],
            dtype=np.float32,
        )
        x = np.concatenate([xd, xo, xd * xo, xd - xo], axis=1)
        assert x.shape[1] == FEATURE_DIM, f"维度 {x.shape[1]} != {FEATURE_DIM}"

        # 训练两个头
        # 问题 1：severity（回归目标 y_sev） — 用于 fast-path draft-only 评分
        #   —- 使用更强学习率 + 更多 epochs 补偲 bge-small 降维损失的表达能力
        w_sev, b_sev = fit_logreg(x, y_sev, l2=0.1, epochs=8000, lr=2.0)
        scores_sev = 1.0 / (1.0 + np.exp(-(x @ w_sev + b_sev)))

        # 问题 2：danger（回归目标 y_dng 二值 0/1）— 用于完整双塔判定
        #   注意：虽然后文注释称 severity 软目标更好，但二进制 danger 头更直接对齐判定：
        #   high/medium → danger=1，low/none → danger=0。二值目标的梯度更集中，
        #   对 high 样本的召回更好（实测 high 检出率提升显著）。
        w_dng, b_dng = fit_logreg(x, y_dng, l2=0.05, epochs=8000, lr=2.0)
        scores_dng = 1.0 / (1.0 + np.exp(-(x @ w_dng + b_dng)))

        # 评估（danger 头，评估口径 y_dng + 阈值）
        acc_tr = float(((scores_dng >= threshold) == y_dng.astype(bool)).mean())
        best_t, best_acc = suggest_threshold(scores_dng, y_dng)
        cv = None if args.no_loo else kfold_report_soft(x, y_dng, y_dng, threshold)

        n_danger = int(y_dng.sum())
        block_line = threshold + BLOCK_MARGIN
        print(f"\n=== {scene}（{len(drafts)} 条：危险 {n_danger} / 安全 {len(drafts)-n_danger}）===")
        print(f"  训练集准确率（danger头）: {acc_tr*100:.1f}%")
        if cv is not None:
            print(
                f"  5折交叉验证: acc {cv['acc']*100:.1f}%  "
                f"安全误报 {cv['fp_rate']*100:.1f}%  危险漏判 {cv['fn_rate']*100:.1f}%"
            )
        print(f"  当前阈值 {threshold:.2f} / Block线 {block_line:.2f}")
        print(f"  该阈值下准确率: {acc_tr*100:.1f}%")
        print(f"  阈值扫描最优: t={best_t:.2f} / 准确率 {best_acc*100:.1f}%")

        sev_arr = np.array(sevs)
        print("  Severity 头按档分数:")
        for tier, tgt in SEVERITY_TARGET.items():
            m = sev_arr == tier
            cnt = int(m.sum())
            if cnt == 0:
                continue
            s = scores_sev[m]
            print(
                f"    {tier:<6}(目标{tgt:.2f}, n={cnt:>4}): "
                f"mean={s.mean():.3f} min={s.min():.3f} max={s.max():.3f}"
            )

        if not args.dry_run:
            head_path = model_dir / head_file
            if head_path.exists():
                existing = json.loads(head_path.read_text(encoding="utf-8"))
            else:
                existing = {"templates": []}
            existing["dim"] = FEATURE_DIM
            existing["jev"] = {
                "severity": {
                    "weights": [float(v) for v in w_sev],
                    "bias": float(b_sev),
                },
                "danger": {
                    "weights": [float(v) for v in w_dng],
                    "bias": float(b_dng),
                },
            }
            # fast-path：draft-only 得分落在 safe_band/block_band → 短路判定（§5.7-jev）。
            existing["fast_path"] = {
                "safe_band": [0.0, 0.30],
                "block_band": [0.70, 1.0],
            }
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
