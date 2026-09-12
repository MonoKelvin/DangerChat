"""OCR 评测（M0-C）。

从 Rust 尖峰导出的清单读取 ROI 图像与真值，测量：
- 聊天标题精确匹配率（门槛 ≥ 99%）；
- 草稿字符错误率 CER（门槛 ≤ 2%）。

清单格式（JSON）：
    {
      "cases": [
        {
          "label": "light-1000x720#项目协作群",
          "title": {"image": "path.png", "expected": "项目协作群"},
          "draft": {"image": "path.png", "expected": "方案已更新，请查收附件"}
        }
      ]
    }

图像由 Rust 端从自有测试窗口捕获并按 ROI 裁剪导出，全部为合成内容。
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from pathlib import Path

from rapidocr_onnxruntime import RapidOCR


@dataclass
class RegionCase:
    label: str
    role: str
    image: Path
    expected: str


@dataclass
class RegionResult:
    label: str
    role: str
    expected: str
    recognized: str
    exact_match: bool
    cer: float
    latency_ms: float


def load_manifest(path: Path) -> list[RegionCase]:
    data = json.loads(path.read_text(encoding="utf-8"))
    cases: list[RegionCase] = []
    for entry in data["cases"]:
        for role in ("title", "draft"):
            region = entry.get(role)
            if region is None:
                continue
            cases.append(
                RegionCase(
                    label=entry["label"],
                    role=role,
                    image=path.parent / region["image"],
                    expected=region["expected"],
                )
            )
    return cases


def normalize(text: str) -> str:
    """产品自身的文本归一化：去除所有空白后再比较。

    依据 docs/02 §12.1，空白与标点归一化是本地规则的一部分；
    对中英文混排文本，空格不改变语义，不应算作识别错误。
    """
    return "".join(text.split())


def character_error_rate(expected: str, recognized: str) -> float:
    """编辑距离 / 真值长度（空白归一化后）。真值为空时按全错处理。"""
    expected = normalize(expected)
    recognized = normalize(recognized)
    if not expected:
        return 0.0 if not recognized else 1.0
    if recognized == expected:
        return 0.0

    # 标准一维滚动数组 Levenshtein。
    previous = list(range(len(expected) + 1))
    for j, rc in enumerate(recognized, start=1):
        current = [j] + [0] * len(expected)
        for i, ec in enumerate(expected, start=1):
            cost = 0 if rc == ec else 1
            current[i] = min(
                previous[i] + 1,        # 删除
                current[i - 1] + 1,     # 插入
                previous[i - 1] + cost,  # 替换
            )
        previous = current
    return previous[len(expected)] / len(expected)


def evaluate(cases: list[RegionCase], engine: RapidOCR) -> list[RegionResult]:
    results: list[RegionResult] = []
    for case in cases:
        outcome, elapsed = engine(case.image.read_bytes())
        text = ""
        if outcome:
            # RapidOCR 返回 [box, text, score] 列表；按行拼接。
            text = "".join(str(line[1]) for line in outcome).strip()
        # elapsed 是各阶段耗时列表（检测/分类/识别），取总和。
        latency = sum(float(x) for x in elapsed) if isinstance(elapsed, list) else float(elapsed)
        results.append(
            RegionResult(
                label=case.label,
                role=case.role,
                expected=case.expected,
                recognized=text,
                exact_match=normalize(text) == normalize(case.expected),
                cer=character_error_rate(case.expected, text),
                latency_ms=latency,
            )
        )
    return results


def run(manifest_path: str) -> int:
    path = Path(manifest_path)
    if not path.is_file():
        print(f"清单不存在: {path}", file=sys.stderr)
        return 1

    cases = load_manifest(path)
    if not cases:
        print("清单为空。", file=sys.stderr)
        return 1

    engine = RapidOCR()
    results = evaluate(cases, engine)

    print("=== M0-C OCR 评测 ===")
    for r in results:
        mark = "OK " if (r.exact_match if r.role == "title" else r.cer <= 0.02) else "ERR"
        print(
            f"  [{mark}] {r.label:<28} {r.role:<5} "
            f"期望={r.expected!r} 识别={r.recognized!r} CER={r.cer:.3f} {r.latency_ms:.0f}ms"
        )

    titles = [r for r in results if r.role == "title"]
    drafts = [r for r in results if r.role == "draft"]
    print()
    if titles:
        rate = sum(r.exact_match for r in titles) / len(titles)
        print(f"标题精确匹配率 = {rate:.3f}（{len(titles)} 个样本，门槛 ≥ 0.99）")
    if drafts:
        worst = max(r.cer for r in drafts)
        print(f"草稿最差 CER   = {worst:.3f}（{len(drafts)} 个样本，门槛 ≤ 0.02）")

    title_ok = titles and sum(r.exact_match for r in titles) / len(titles) >= 0.99
    draft_ok = drafts and max(r.cer for r in drafts) <= 0.02
    print()
    print(f"门槛 标题精确匹配 ≥ 99% : {'PASS' if title_ok else 'FAIL'}")
    print(f"门槛 草稿 CER ≤ 2%      : {'PASS' if draft_ok else 'FAIL'}")
    return 0 if (title_ok and draft_ok) else 1


def main(argv: list[str]) -> int:
    if len(argv) >= 3 and argv[1] == "ocr-eval":
        return run(argv[2])
    print("用法: python -m dc_worker ocr-eval <manifest.json>", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
