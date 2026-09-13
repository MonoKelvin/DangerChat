"""M3：语义模型 fp32 vs int8 准确率对照与阈值校准（设计文档 §5.7、§8）。

要回答两个问题：
  1. int8 量化后，场景匹配的判别能力是否还够用（不是比「输出张量总和」，而是比**判别结果**）？
  2. 阈值该怎么定？（直接决定 `sem.threshold.formal` / `sem.threshold.casual` 的取值）

方法：
  · 句向量 = mean-pool(last_hidden_state) → L2 归一化（BGE 官方推荐做法）
  · 场景模板各取一段描述文本，草稿与模板做余弦相似度
  · 「正式场景」下 sim(draft, formal) 低于阈值 → 判危险
  · 用同一套带标注样本，分别给 fp32 / int8 求最优阈值与准确率；再看「沿用 fp32 阈值」时 int8 的表现

用法：
    python tools/training/calibrate_sem.py spikes/m3-inference/models/bge
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

# ---- 场景模板（可配置项 sem.scenario_templates 的候选文案）----
TEMPLATES = {
    "formal": "这是与客户、领导或公司工作群组的正式沟通，需要保持专业、礼貌、克制。",
    "casual": "这是与好朋友或家人的私下闲聊，语气随意、放松，可以吐槽抱怨。",
}

# ---- 危险/安全样例句集（比对「单模板相似度」与「样例最大相似度」两种打分法）----
DANGER_EXEMPLARS = [
    "这破需求一天改八遍，我真服了",
    "傻逼领导，天天让加班还不给钱",
    "这公司烂透了，我早就想跑路",
    "烦死了，又要陪客户应酬到半夜",
    "工资这么点还让人干这么多活",
    "甲方纯粹有病，我要被逼疯了",
]
SAFE_EXEMPLARS = [
    "好的，我下午把方案发您",
    "收到，明天上午同步进度",
    "张总，附件是本周报表，请过目",
    "会议纪要整理好了，稍后发群里",
    "您好，方便沟通一下合同细节吗",
]

# BGE-zh v1.5 官方建议的查询指令前缀（查询→段落模式）
BGE_QUERY_PREFIX = "为这个句子生成表示以用于检索相关文章："

# ---- 标注样本：(草稿, 在正式场景下是否安全) ----
SAMPLES: list[tuple[str, bool]] = [
    ("好的，我下午把这个方案发您", True),
    ("收到，明天上午十点前同步进度", True),
    ("张总，附件是本周的销售报表，请您过目", True),
    ("这个需求我评估一下，明天给您答复", True),
    ("会议纪要已经整理好了，稍后发到群里", True),
    ("您好，请问方便沟通一下合同细节吗", True),
    ("我已经按照您的意见修改了第二版", True),
    ("这版设计稿的交付时间是本周五", True),
    ("有问题随时找我，我随时在线", True),
    ("辛苦了，这个项目推进得很顺利", True),
    ("这破需求一天改八遍，我真服了", False),
    ("傻逼领导，天天让加班还不给钱", False),
    ("这项目烂透了，我早就想跑路了", False),
    ("烦死了，又要陪客户吃饭到半夜", False),
    ("公司这群人一个比一个能装", False),
    ("工资这么点还让人干这么多活，无语", False),
    ("我要被这个甲方逼疯了，纯粹有病", False),
    ("算了不说了，说多了都是泪，麻了", False),
    ("这帮人做事真的离谱，血压上来了", False),
    ("真的受够了，明天就想提离职", False),
]


def make_session(path: Path, cuda_free: bool = True) -> ort.InferenceSession:
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 2  # 与 §2.3 的线程纪律一致
    providers = ["CPUExecutionProvider"]
    if not cuda_free:  # 保留参数位：需要时改成 DmlExecutionProvider
        providers = ["DmlExecutionProvider", "CPUExecutionProvider"]
    return ort.InferenceSession(str(path), sess_options=opts, providers=providers)


def embed(session: ort.InferenceSession, tok: Tokenizer, texts: list[str]) -> np.ndarray:
    """mean-pooling + L2 归一化。"""
    out = []
    for text in texts:
        enc = tok.encode(text)
        ids = np.array([enc.ids], dtype=np.int64)
        mask = np.array([enc.attention_mask], dtype=np.int64)
        types = np.zeros_like(ids)
        feeds = {
            "input_ids": ids,
            "attention_mask": mask,
            "token_type_ids": types,
        }
        # 只喂模型真正声明的输入（不同导出可能缺 token_type_ids）
        feeds = {k: v for k, v in feeds.items() if k in {i.name for i in session.get_inputs()}}
        hidden = session.run(None, feeds)[0]  # [1, seq, 512]
        m = mask[0][:, None].astype(np.float32)
        vec = (hidden[0] * m).sum(axis=0) / np.clip(m.sum(), 1e-9, None)
        vec = vec / np.linalg.norm(vec)
        out.append(vec)
    return np.array(out)


def embed_prefixed(session: ort.InferenceSession, tok: Tokenizer, texts: list[str], prefix: str = "") -> np.ndarray:
    """带查询前缀的编码（BGE 查询→段落模式）。"""
    return embed(session, tok, [prefix + t if prefix else t for t in texts])


def best_threshold(sims: np.ndarray, labels: np.ndarray) -> tuple[float, float]:
    """在 sim 上扫阈值：sim >= t 判安全，返回 (最优阈值, 准确率)。"""
    best_t, best_acc = 0.0, 0.0
    for t in np.arange(0.20, 0.85, 0.01):
        acc = float(((sims >= t) == labels).mean())
        if acc > best_acc:
            best_t, best_acc = float(t), acc
    return best_t, best_acc


def loo_probe(x: np.ndarray, y: np.ndarray, l2: float = 0.5, epochs: int = 800, lr: float = 0.5) -> float:
    """留一交叉验证的逻辑回归探针（纯 numpy）。

    回答的问题不是「阈值是多少」，而是「嵌入空间里**有没有**这层信息」：
    若 LOO 准确率高，说明 §5.7 只需把「模板相似度」换成「训练过的线性头」；
    若依然接近随机，则 bge-small-zh 对语域/情绪这个任务不适用，需要换模型。
    """
    n = x.shape[0]
    correct = 0
    for held in range(n):
        train_idx = [i for i in range(n) if i != held]
        xt, yt = x[train_idx], y[train_idx]
        w = np.zeros(x.shape[1])
        b = 0.0
        for _ in range(epochs):
            z = xt @ w + b
            pr = 1.0 / (1.0 + np.exp(-z))
            w -= lr * (xt.T @ (pr - yt) / len(yt) + l2 * w / len(yt))
            b -= lr * float((pr - yt).mean())
        pred = float(x[held] @ w + b) >= 0.0
        correct += int(pred == bool(y[held]))
    return correct / n


def main() -> int:
    model_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "spikes/m3-inference/models/bge")
    tok = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
    drafts = [s[0] for s in SAMPLES]
    labels = np.array([s[1] for s in SAMPLES])

    results = {}
    for name, file in [("fp32", "model.onnx"), ("int8", "model_quantized.onnx")]:
        path = model_dir / file
        if not path.exists():
            print(f"跳过 {name}：{path} 不存在")
            continue
        sess = make_session(path)
        size_mb = path.stat().st_size / 1e6

        # 打分法 A：与「正式场景模板」的相似度（现行 §5.7 设计）
        formal_tmpl = embed(sess, tok, [TEMPLATES["formal"]])[0]
        sims_template = embed(sess, tok, drafts) @ formal_tmpl
        t_a, acc_a = best_threshold(sims_template, labels)

        # 打分法 B：与「危险样例」的最大相似度（查询带指令前缀）
        danger_vecs = embed_prefixed(sess, tok, DANGER_EXEMPLARS, BGE_QUERY_PREFIX)
        safe_vecs = embed_prefixed(sess, tok, SAFE_EXEMPLARS, BGE_QUERY_PREFIX)
        query_vecs = embed_prefixed(sess, tok, drafts, BGE_QUERY_PREFIX)
        sims_exemplar = (query_vecs @ danger_vecs.T).max(axis=1)
        t_b, acc_b = best_threshold(sims_exemplar, labels)

        # 打分法 C：危险最大相似度 − 安全最大相似度（对比式）
        margin = (query_vecs @ danger_vecs.T).max(axis=1) - (query_vecs @ safe_vecs.T).max(axis=1)
        t_c, acc_c = best_threshold(margin, labels)

        results[name] = {
            "sims": sims_template,
            "threshold": t_a,
            "acc": acc_a,
            "size_mb": size_mb,
            "vecs": query_vecs,
            "vecs_danger": danger_vecs,
            "exemplar_sims": sims_exemplar,
            "exemplar_acc": acc_b,
            "exemplar_threshold": t_b,
            "margin_acc": acc_c,
            "margin_threshold": t_c,
        }
        print(
            f"{name:>4}  体积 {size_mb:6.2f} MB   "
            f"A 模板相似度: 阈值 {t_a:.2f} / 准确率 {acc_a * 100:3.0f}%   "
            f"B 危险样例最大相似度: 阈值 {t_b:.2f} / 准确率 {acc_b * 100:3.0f}%   "
            f"C 危险−安全差值: 阈值 {t_c:.2f} / 准确率 {acc_c * 100:3.0f}%"
        )

    # ---- 嵌入质量自检：模型与流水线本身是否正常 ----
    print("\n== 自检：同句相似度与近邻关系 ==")
    if "fp32" in results and results["fp32"].get("vecs") is not None:
        v = results["fp32"]["vecs"]
        self_sim = float((v[0] * v[0]).sum())
        cross = float((v[0] * v[1]).sum())
        print(f"  sim(同句) = {self_sim:.3f}（应为 1.0）   sim(两个安全句) = {cross:.3f}")
        d_i = float((v[10] * results["fp32"]["vecs_danger"][0]).sum()) if "vecs_danger" in results["fp32"] else float("nan")
        print(f"  sim(危险句, 危险样例) = {d_i:.3f}")

    # ---- 线性探针：嵌入空间是否**线性可分**（余弦对模板不行 ≠ 没有信息）----
    print("\n== 线性探针（留一交叉验证，纯 numpy 逻辑回归）==")
    for name in ("fp32", "int8"):
        if name not in results or results[name].get("vecs") is None:
            continue
        X, y = results[name]["vecs"], labels.astype(np.float64)
        acc = loo_probe(X, y)
        results[name]["probe_acc"] = acc
        print(f"  {name:>4}: LOO 准确率 {acc * 100:3.0f}%")

    if "fp32" in results and "int8" in results:
        f, q = results["fp32"], results["int8"]
        # 排序一致性：两种精度是否给出相同的危险排序
        order_f = np.argsort(f["sims"])
        order_q = np.argsort(q["sims"])
        rank_agree = float((order_f == order_q).mean())
        # 沿用 fp32 阈值时，int8 的准确率变化
        acc_same = float(((q["sims"] >= f["threshold"]) == labels).mean())
        print()
        print(f"排序一致性（两种精度的相似度序）     : {rank_agree * 100:.0f}%")
        print(f"沿用 fp32 阈值 {f['threshold']:.2f} 时 int8 准确率 : {acc_same * 100:.0f}%")
        print(f"int8 自身最优阈值 {q['threshold']:.2f}（与 fp32 相差 {abs(q['threshold'] - f['threshold']):.2f}）")
        print(f"体积压缩                            : {f['size_mb'] / q['size_mb']:.2f}×")

        print("\n逐条相似度（越高越像正式场景，安全样本应高于危险样本）：")
        print(f"  {'标签':<4} {'fp32':>7} {'int8':>7}  草稿")
        for (text, safe), sf, sq in zip(SAMPLES, f["sims"], q["sims"]):
            tag = "安全" if safe else "危险"
            print(f"  {tag:<4} {sf:7.3f} {sq:7.3f}  {text}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
