"""训练 jev 风格多问题线性头（ADR-14 增强版）。

相比 train_head.py 的改动：
1. **编码器换为 bge-small**（512 维，满足 FR-SEM-05 延迟预算）；
2. **标记位置读取**：在 [CLS] 后插入 [unused1]，读取位置 1 的隐藏状态（不做 mean-pooling）；
3. **多问题设计**：
   - 问题 1（severity Score）：消息严重程度 none/low/medium/high → [0, 0.2, 0.65, 0.97]
   - 问题 2（danger Noul）：是否危险 true/false → [0, 1]
   两个问题共用同一 512 维嵌入，分别训练两个线性头（w1+b1, w2+b2）。
4. **特征简化**：单塔 512 维（不再拼接对象塔/交互项）——jev 原论文主张将对象信息编码到
   输入文本里，避免显式拼接导致的可分离性问题。

语料：resources/training_set/conversation/<category>/*.jsonl（与 train_head.py 共用）。
      行 schema 见 schema.json，核心字段：text（嵌入串）、danger（bool）、severity、base。

用法：
    python tools/training/train_jev_head.py
    python tools/training/train_jev_head.py --dry-run
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

# 运行时阈值（对照 crates/dc-pipeline/src/sem/mod.rs）
THRESHOLD_FORMAL = 0.55
THRESHOLD_CASUAL = 0.45
BLOCK_MARGIN = 0.25
EMBED_DIM = 512  # bge-small-zh（切换自 bge-large 1024）

# severity 软目标（与 train_head.py 一致）
SEVERITY_TARGET = {"none": 0.0, "low": 0.20, "medium": 0.65, "high": 0.97}
LEGACY_SOFT_TRUE = SEVERITY_TARGET["medium"]
LEGACY_SOFT_FALSE = SEVERITY_TARGET["none"]
SOFT_CONFLICT_GAP = 0.5

BASES = ("formal", "casual")
REPO_ROOT = Path(__file__).resolve().parents[2]

# 标记 ID（[unused1]）
MARKER_ID = 1


def embed_at_marker(
    session: ort.InferenceSession, tok: Tokenizer, text: str, marker_id: int = MARKER_ID
) -> np.ndarray:
    """标记位置读取：[CLS] marker_id text... [SEP]，返回位置 1 的隐藏状态（L2 归一化）。"""
    enc = tok.encode(text)
    # [CLS]=101 + marker + 其余内容（截断到 128）
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

    hidden = session.run(None, feeds)[0]  # [1, seq, 512]
    vec = hidden[0, 1, :]  # 位置 1（标记位置）
    norm = np.linalg.norm(vec)
    if norm < 1e-9:
        raise ValueError("嵌入退化（零向量）")
    vec = vec / norm
    return vec.astype(np.float32)


def _row_targets(row: dict) -> tuple[float, int, str]:
    """解析（soft_severity, binary_danger, severity_tier）。"""
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
) -> dict[str, tuple[list[str], np.ndarray, np.ndarray, list[str]]]:
    """按 base 分桶：{base: (texts, y_severity_soft, y_danger_binary, severities)}。"""
    buckets: dict[str, dict[str, tuple[float, int, str]]] = {b: {} for b in BASES}
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
                soft, y_danger, sev = _row_targets(row)
            except ValueError as e:
                raise SystemExit(f"{f}:{lineno} {e}")
            # jev 单塔：将对象信息编码到文本中（对照 §5.7 原双塔设计）
            obj = row.get("object") or "未标记"
            # 输入格式：{row['text']}\n对象：{obj}
            # 注：row['text'] 已包含"之前的对话：{ctx}\n现在要发送：{draft}"格式，
            # 不需要重复包装。对象信息作为额外上下文段落追加。
            text = f"{row['text']}\n对象：{obj}"
            if text in buckets[base]:
                if abs(buckets[base][text][0] - soft) > SOFT_CONFLICT_GAP:
                    raise SystemExit(
                        f"{f}:{lineno} 标签冲突：重复文本软目标差 > {SOFT_CONFLICT_GAP}"
                    )
                continue
            buckets[base][text] = (soft, y_danger, sev)

    out: dict[str, tuple[list[str], np.ndarray, np.ndarray, list[str]]] = {}
    for base, seen in buckets.items():
        texts = list(seen.keys())
        y_sev = np.array([seen[t][0] for t in texts], dtype=np.float64)
        y_dng = np.array([seen[t][1] for t in texts], dtype=np.float64)
        sevs = [seen[t][2] for t in texts]
        out[base] = (texts, y_sev, y_dng, sevs)
    total = sum(len(t) for t, *_ in out.values())
    print(f"语料：{total} 条（去重后）")
    for base in BASES:
        t, _, yb, sevs = out[base]
        if len(t):
            c = Counter(sevs)
            print(
                f"  [{base}] {len(t)} 条：危险 {int(yb.sum())} / 安全 {int((1 - yb).sum())}"
                f"  ｜ severity none={c['none']} low={c['low']} medium={c['medium']} high={c['high']}"
            )
    print()
    return out


def fit_logreg(
    x: np.ndarray, y: np.ndarray, l2: float = 0.5, epochs: int = 2000, lr: float = 0.5
) -> tuple[np.ndarray, float]:
    """逻辑回归（float32）。"""
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
    """k 折交叉验证（训练用 y_soft，评估用 y_bool + threshold）。"""
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
    """扫阈值：score>=t 判危险，返回 (最优阈值, 准确率)。"""
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
    ap.add_argument("--dry-run", action="store_true", help="只训练打印，不写 head 文件")
    ap.add_argument("--no-loo", action="store_true", help="跳过交叉验证")
    args = ap.parse_args()

    model_dir = Path(args.model_dir)
    corpus_dir = Path(args.corpus)
    tok = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
    weight = model_dir / "model_quantized.onnx"
    sess = ort.InferenceSession(str(weight), providers=["CPUExecutionProvider"])
    print(f"模型：{weight}（bge-small-zh int8）\n")

    corpus = load_corpus(corpus_dir)

    scenes = [
        ("formal", "head-formal.json", THRESHOLD_FORMAL),
        ("casual", "head-casual.json", THRESHOLD_CASUAL),
    ]

    for scene, head_file, threshold in scenes:
        texts, y_sev, y_dng, sevs = corpus[scene]
        if len(texts) == 0:
            print(f"[{scene}] 无语料，跳过")
            continue

        # 特征提取：单塔 jev 标记读取
        print(f"[{scene}] 提取 {len(texts)} 条特征...")
        x = np.array([embed_at_marker(sess, tok, t) for t in texts], dtype=np.float32)
        assert x.shape[1] == EMBED_DIM, f"维度 {x.shape[1]} != {EMBED_DIM}"

        # 训练两个头
        # 问题 1：severity（回归目标 y_sev） — 用于 fast-path draft-only 评分
        w_sev, b_sev = fit_logreg(x, y_sev, l2=0.1, epochs=8000, lr=2.0)
        scores_sev = 1.0 / (1.0 + np.exp(-(x @ w_sev + b_sev)))

        # 问题 2：danger（回归目标 y_dng 二值 0/1）— 用于完整判定
        w_dng, b_dng = fit_logreg(x, y_dng, l2=0.05, epochs=8000, lr=2.0)
        scores_dng = 1.0 / (1.0 + np.exp(-(x @ w_dng + b_dng)))

        # 评估（danger 头，评估口径 y_dng + 阈值）
        acc_tr = float(((scores_dng >= threshold) == y_dng.astype(bool)).mean())
        best_t, best_acc = suggest_threshold(scores_dng, y_dng)
        cv = None if args.no_loo else kfold_report_soft(x, y_dng, y_dng, threshold)

        n_danger = int(y_dng.sum())
        block_line = threshold + BLOCK_MARGIN
        print(f"\n=== {scene}（{len(texts)} 条：危险 {n_danger} / 安全 {len(texts)-n_danger}）===")
        print(f"  训练集准确率（danger头）: {acc_tr*100:.1f}%")
        if cv is not None:
            print(
                f"  5折交叉验证: acc {cv['acc']*100:.1f}%  "
                f"安全误报 {cv['fp_rate']*100:.1f}%  危险漏判 {cv['fn_rate']*100:.1f}%"
            )
        print(f"  当前阈值 {threshold:.2f} / Block线 {block_line:.2f}")
        print(f"  该阈值下准确率: {acc_tr*100:.1f}%")
        print(f"  阈值扫描最优: t={best_t:.2f} / 准确率 {best_acc*100:.1f}%")

        # 按 severity 档打印分数（severity 头）
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
            # 写入两个头（jev 多问题）
            existing["dim"] = EMBED_DIM
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
