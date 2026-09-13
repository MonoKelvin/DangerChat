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
from time import perf_counter

from rapidocr_onnxruntime import RapidOCR

from .ocr_serve import DET_LIMIT_SIDE_LEN


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
    edit_distance: int
    expected_chars: int
    cer: float
    latency_ms: float


class ManifestError(ValueError):
    """评测清单不符合本地、相对路径的数据契约。"""


def _require_string(value: object, field: str) -> str:
    if not isinstance(value, str):
        raise ManifestError(f"{field} 必须是字符串")
    return value


def _resolve_image(root: Path, value: object, field: str) -> Path:
    image = Path(_require_string(value, field))
    if image.is_absolute() or ".." in image.parts:
        raise ManifestError(f"{field} 必须是清单目录内的相对路径")
    resolved = (root / image).resolve()
    try:
        resolved.relative_to(root.resolve())
    except ValueError as exc:
        raise ManifestError(f"{field} 逃逸清单目录") from exc
    if not resolved.is_file():
        raise ManifestError(f"{field} 文件不存在: {image}")
    return resolved


def load_manifest(path: Path) -> list[RegionCase]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ManifestError(f"无法读取清单: {exc}") from exc
    if not isinstance(data, dict) or not isinstance(data.get("cases"), list):
        raise ManifestError("清单根对象必须包含 cases 数组")

    cases: list[RegionCase] = []
    for index, entry in enumerate(data["cases"]):
        if not isinstance(entry, dict):
            raise ManifestError(f"cases[{index}] 必须是对象")
        label = _require_string(entry.get("label"), f"cases[{index}].label")
        for role in ("title", "draft"):
            region = entry.get(role)
            if region is None:
                continue
            if not isinstance(region, dict):
                raise ManifestError(f"cases[{index}].{role} 必须是对象")
            prefix = f"cases[{index}].{role}"
            cases.append(
                RegionCase(
                    label=label,
                    role=role,
                    image=_resolve_image(path.parent, region.get("image"), f"{prefix}.image"),
                    expected=_require_string(region.get("expected"), f"{prefix}.expected"),
                )
            )
    return cases


def normalize(text: str) -> str:
    """产品自身的文本归一化：去除所有空白后再比较。

    依据 docs/02 §12.1，空白与标点归一化是本地规则的一部分；
    对中英文混排文本，空格不改变语义，不应算作识别错误。
    """
    return "".join(text.split())


def edit_distance(expected: str, recognized: str) -> tuple[int, int]:
    """返回空白归一化后的（Levenshtein 距离，真值字符数）。"""
    expected = normalize(expected)
    recognized = normalize(recognized)
    if recognized == expected:
        return 0, len(expected)

    previous = list(range(len(expected) + 1))
    for j, rc in enumerate(recognized, start=1):
        current = [j] + [0] * len(expected)
        for i, ec in enumerate(expected, start=1):
            cost = 0 if rc == ec else 1
            current[i] = min(
                previous[i] + 1,  # 删除
                current[i - 1] + 1,  # 插入
                previous[i - 1] + cost,  # 替换
            )
        previous = current
    return previous[len(expected)], len(expected)


def character_error_rate(expected: str, recognized: str) -> float:
    """编辑距离 / 真值长度；空真值只有在识别也为空时为零。"""
    distance, expected_chars = edit_distance(expected, recognized)
    if expected_chars == 0:
        return 0.0 if distance == 0 else 1.0
    return distance / expected_chars


def corpus_character_error_rate(results: list[RegionResult]) -> float:
    """按总编辑距离/总真值字符数计算语料级 CER。"""
    total_chars = sum(result.expected_chars for result in results)
    total_distance = sum(result.edit_distance for result in results)
    if total_chars == 0:
        return 0.0 if total_distance == 0 else 1.0
    return total_distance / total_chars


def evaluate(cases: list[RegionCase], engine: RapidOCR) -> list[RegionResult]:
    results: list[RegionResult] = []
    for case in cases:
        started = perf_counter()
        outcome, _ = engine(case.image.read_bytes())
        # RapidOCR 的阶段耗时以秒计，且无文字时可能是 None。
        # 统一测量包含读图、预处理和后处理的实际墙钟时间。
        latency = (perf_counter() - started) * 1000.0
        text = ""
        if outcome:
            # RapidOCR 返回 [box, text, score] 列表；按行拼接。
            text = "".join(str(line[1]) for line in outcome).strip()
        distance, expected_chars = edit_distance(case.expected, text)
        cer = 0.0 if expected_chars == 0 and distance == 0 else (
            1.0 if expected_chars == 0 else distance / expected_chars
        )
        results.append(
            RegionResult(
                label=case.label,
                role=case.role,
                expected=case.expected,
                recognized=text,
                exact_match=normalize(text) == normalize(case.expected),
                edit_distance=distance,
                expected_chars=expected_chars,
                cer=cer,
                latency_ms=latency,
            )
        )
    return results


def run(manifest_path: str) -> int:
    path = Path(manifest_path)
    if not path.is_file():
        print(f"清单不存在: {path}", file=sys.stderr)
        return 1

    try:
        cases = load_manifest(path)
    except ManifestError as exc:
        print(f"清单无效: {exc}", file=sys.stderr)
        return 1
    if not cases:
        print("清单为空。", file=sys.stderr)
        return 1

    try:
        # 与 ocr_serve 使用同一检测参数，保证质量结论对应实际运行配置。
        engine = RapidOCR(
            intra_op_num_threads=2,
            inter_op_num_threads=1,
            det_limit_side_len=DET_LIMIT_SIDE_LEN,
        )
        results = evaluate(cases, engine)
    except Exception as exc:
        print(f"OCR 评测失败（{type(exc).__name__}），未产生有效门槛结论。", file=sys.stderr)
        return 1

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
    draft_cer = corpus_character_error_rate(drafts)
    if drafts:
        worst = max(r.cer for r in drafts)
        print(
            f"草稿语料 CER   = {draft_cer:.3f}（{len(drafts)} 个样本，"
            f"最差样本 {worst:.3f}，门槛 ≤ 0.02）"
        )

    title_ok = bool(titles) and sum(r.exact_match for r in titles) / len(titles) >= 0.99
    draft_ok = bool(drafts) and draft_cer <= 0.02
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
