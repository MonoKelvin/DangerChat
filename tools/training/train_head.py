"""训练 §5.7 场景线性头（ADR-14）：把标注语料嵌入后拟合逻辑回归，
产出 `weights` + `bias` 写入 resources/models/bge/head-{formal,casual}.json。

背景：head 文件此前只有 templates（模板兜底，只提示不拦截，准确率≈随机）。
本脚本产出真实线性头，激活 L2 拦截（score = sigmoid(w·embed + b)）。

**回归目标是 severity 软标签，而非 danger 二值**：danger 只有 0/1，会把 low（平庸不合场景、
无实害）与 high（严重后果）压到同一侧，模型学不到「平庸 < 严重」的连续梯度，平庸消息也易越
阈值误报。改用 SEVERITY_TARGET 把 none/low/medium/high 映射到 [0,1] 连续目标后，sigmoid 回归
分数天然按 severity 分层——平庸落在阈值下（放行）、严重稳超 Block 线（硬拦）。评估口径仍是
danger 二值（误报/漏判统计）。

语料：resources/training_set/conversation/<category>/*.jsonl（category 最多一层）。
      行 schema 见同目录 schema.json，核心字段：
        {"text": 运行时嵌入串, "danger": bool, "severity": none|low|medium|high, "base": formal|casual}
      训练目标 = SEVERITY_TARGET[severity]；legacy 无 severity 字段时按 danger 回落。
      评估标签 = danger（= severity∈{medium,high}）。
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
from collections import Counter
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

# 与运行时保持一致（crates/dc-pipeline/src/sem/mod.rs::THRESHOLD_*、
# crates/dc-pipeline/src/verdict.rs::BLOCK_MARGIN）
THRESHOLD_FORMAL = 0.55
THRESHOLD_CASUAL = 0.45
BLOCK_MARGIN = 0.25   # 仅用于打印 Block 线；实际扣分在 verdict.rs::from_score
EMBED_DIM = 1024         # 单塔维度（= embedder.rs::EMBED_DIM，bge-large-zh）
FEATURE_DIM = 4 * EMBED_DIM   # 草稿塔 ⊕ 对象塔 ⊕ 逐元素积 ⊕ 差（含交互项）

# severity → 软目标（回归目标 y ∈ [0,1]）。
# 为何回归而非二值分类：danger 只有 0/1，把 low（平庸不合场景，无实害）和 high（严重后果）
# 压到同一侧，模型学不到「平庸 < 严重」的梯度，平庸消息也易越阈值误报。改用连续软目标后，
# sigmoid 回归的梯度 x.T@(pr-y) 对连续 y 依然成立（等价软标签交叉熵），分数天然按 severity 分层。
# 取值锚定运行时阈值语义（formal 0.55 / casual 0.45，Block 线 = 阈值 + 0.25 = 0.80 / 0.70）：
#   none 0.00 / low 0.20（远低于阈值 → Safe，平庸放行）
#   medium 0.65（越阈值但不到 Block 线 → Warn 提示）
#   high 0.97（稳超 Block 线 → Block 硬拦）
SEVERITY_TARGET = {"none": 0.0, "low": 0.20, "medium": 0.65, "high": 0.97}
# legacy 样本（无 severity 字段）按 danger 回落的软目标：danger=true 保守取 medium 档，
# danger=false 取 none。纠偏补全 severity 后此回落基本不触发，仅作过渡兜底。
LEGACY_SOFT_TRUE = SEVERITY_TARGET["medium"]
LEGACY_SOFT_FALSE = SEVERITY_TARGET["none"]
# 同一 (text, 对象) 软目标差超过该值才判标签冲突（噪声）；
# medium/high 之间的细微差异属正常分级，不算冲突。
SOFT_CONFLICT_GAP = 0.5

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


def _row_targets(row: dict) -> tuple[float, int, str]:
    """从一行语料解析（软目标 y_soft, 评估标签 y_bool, severity 档名）。

    - y_soft：回归目标。有 severity 字段 → 查 SEVERITY_TARGET；legacy 无字段 → 按 danger 回落。
    - y_bool：评估口径，恒等于 danger（= severity∈{medium,high}），用于误报/漏判统计。
    - sev：severity 档名（none/low/medium/high），legacy 无字段时按 danger 记为 medium/none 便于分档打印。
    """
    danger = bool(row["danger"])
    sev = row.get("severity")
    if sev is not None:
        if sev not in SEVERITY_TARGET:
            raise ValueError(f"severity 非法：{sev!r}")
        return SEVERITY_TARGET[sev], (1 if danger else 0), sev
    # legacy 兜底：无 severity 字段
    soft = LEGACY_SOFT_TRUE if danger else LEGACY_SOFT_FALSE
    return soft, (1 if danger else 0), ("medium" if danger else "none")


def load_corpus(
    corpus_dir: Path,
) -> dict[str, tuple[list[str], list[str], np.ndarray, np.ndarray, list[str]]]:
    """递归读取语料并按 `base` 基线分桶（formal / casual 两个头）。

    - 目录：`corpus_dir/<category>/*.jsonl`，category 最多一层；
    - 路由：每行 `base` 字段；缺省回落所属目录名（兼容 formal/ casual/ 基线桶目录）；
    - 去重/冲突：键为 **(text, 对象塔文本)**。配对样本（同一草稿 × 不同对象、方向相反）
      是刻意设计，必须允许；只有同一 (text, 对象) 软目标差 > SOFT_CONFLICT_GAP 才算噪声并报错
      （medium/high 之间的细微分级差异不算冲突）。

    返回 `{base: (draft_texts, object_texts, y_soft, y_bool, severities)}`，各列表一一对应。
    """
    # key -> (y_soft, y_bool, sev)
    buckets: dict[str, dict[tuple[str, str], tuple[float, int, str]]] = {b: {} for b in BASES}
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
            try:
                soft, y_bool, sev = _row_targets(row)
            except ValueError as e:
                raise SystemExit(f"{f}:{lineno} {e}")
            obj = "对象：" + (row.get("object") or "未标记")   # 对象塔输入
            key = (row["text"], obj)
            seen = buckets[base]
            if key in seen:
                if abs(seen[key][0] - soft) > SOFT_CONFLICT_GAP:
                    raise SystemExit(
                        f"{f}:{lineno} 标签冲突：同一 base 内 (text, 对象) 重复且"
                        f"软目标差 > {SOFT_CONFLICT_GAP}（{seen[key][0]:.2f} vs {soft:.2f}）"
                    )
                continue  # 重复（含 medium/high 细微差异）→ 保留首次，跳过
            seen[key] = (soft, y_bool, sev)
            kept += 1
        stats.append((f"{f.parent.name}/{f.name}", kept))

    out: dict[str, tuple[list[str], list[str], np.ndarray, np.ndarray, list[str]]] = {}
    for base, seen in buckets.items():
        keys = list(seen.keys())
        texts = [k[0] for k in keys]
        objs = [k[1] for k in keys]
        y_soft = np.array([seen[k][0] for k in keys], dtype=np.float64)
        y_bool = np.array([seen[k][1] for k in keys], dtype=np.float64)
        sevs = [seen[k][2] for k in keys]
        out[base] = (texts, objs, y_soft, y_bool, sevs)
    total = sum(len(t) for t, *_ in out.values())
    print(f"语料：{len(stats)} 个文件 / {total} 条（去重后）")
    for base in BASES:
        t, _, _, yb, sevs = out[base]
        if len(t):
            c = Counter(sevs)
            print(
                f"  [{base}] {len(t)} 条：危险 {int(yb.sum())} / 安全 {int((1 - yb).sum())}"
                f"  ｜ severity none={c['none']} low={c['low']} medium={c['medium']} high={c['high']}"
            )
    print()
    return out


def fit_logreg(x: np.ndarray, y: np.ndarray, l2: float = 0.5,
               epochs: int = 2000, lr: float = 0.5) -> tuple[np.ndarray, float]:
    """纯 numpy 逻辑回归（与 calibrate_sem.loo_probe 同内核）。返回 (w, b)。

    全程 float32：4096 维特征 × 近千样本，float64 会让每轮 `x @ w` 把整个矩阵
    上采样为 float64（翻倍内存 + 碎片），在 311MB ONNX arena 之上易触发 OOM。
    """
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


def kfold_report_soft(x: np.ndarray, y_soft: np.ndarray, y_bool: np.ndarray,
                      threshold: float, k: int = 5, l2: float = 0.5, seed: int = 42) -> dict:
    """软目标版 k 折：训练拟合连续 y_soft，留出集按二值 y_bool + 运行时阈值统计。

    与 kfold_report 的唯一区别是训练目标是 severity 软标签，评估口径仍是 danger 二值
    （安全误报 = 平庸/安全被判危险，危险漏判 = 严重后果被放行）。
    """
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
    ap.add_argument("--model-dir", default=str(REPO_ROOT / "resources/models/bge-large"))
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
        texts, objs, y_soft, y_bool, sevs = corpus[scene]
        if len(texts) == 0:
            print(f"[{scene}] 无语料，跳过")
            continue
        # 特征装配顺序必须与 sem/mod.rs::assemble_features 一致（改动即需同步两者）
        xd = embed(sess, tok, texts)
        xo = embed(sess, tok, objs)
        x = np.concatenate([xd, xo, xd * xo, xd - xo], axis=1)
        assert x.shape[1] == FEATURE_DIM, f"特征维度 {x.shape[1]} != {FEATURE_DIM}"
        # 训练用连续软目标（severity 分层）；评估用二值 danger（误报/漏判口径）
        w, b = fit_logreg(x, y_soft)
        scores = 1.0 / (1.0 + np.exp(-(x @ w + b)))

        tr_acc = train_accuracy(x, y_bool, w, b)
        best_t, best_acc = suggest_threshold(scores, y_bool)
        block_line = threshold + BLOCK_MARGIN
        acc_at_threshold = float(((scores >= threshold) == y_bool.astype(bool)).mean())
        cv = None if args.no_loo else kfold_report_soft(x, y_soft, y_bool, threshold)

        n_danger = int(y_bool.sum())
        print(f"=== {scene}（{len(texts)} 条：危险 {n_danger} / 安全 {len(texts)-n_danger}）===")
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
        print(f"  危险类分数 mean={scores[y_bool==1].mean():.3f} min={scores[y_bool==1].min():.3f}")
        print(f"  安全类分数 mean={scores[y_bool==0].mean():.3f} max={scores[y_bool==0].max():.3f}")
        # 按 severity 4 档打印分数均值：核对 low/none 是否被压到阈值下、high 是否稳超 Block 线
        sev_arr = np.array(sevs)
        print("  按 severity 分档 score:")
        for tier, tgt in SEVERITY_TARGET.items():
            m = sev_arr == tier
            cnt = int(m.sum())
            if cnt == 0:
                continue
            s = scores[m]
            print(
                f"    {tier:<6}(目标{tgt:.2f}, n={cnt:>4}): "
                f"mean={s.mean():.3f} min={s.min():.3f} max={s.max():.3f}"
            )

        if not args.dry_run:
            head_path = model_dir / head_file
            # 已有 head 文件则保留其 templates（兜底相似度锚点）；首次训练无文件则新建。
            if head_path.exists():
                existing = json.loads(head_path.read_text(encoding="utf-8"))
            else:
                existing = {"templates": []}
            # dim=4096（4×1024）→ 运行时启用 草稿⊕对象⊕积⊕差（sem/head.rs 接受 1×/2×/4×）
            existing["dim"] = FEATURE_DIM
            existing["weights"] = [float(v) for v in w]
            existing["bias"] = float(b)
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
