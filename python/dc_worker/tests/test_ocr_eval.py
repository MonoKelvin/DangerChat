"""CER 计算与清单解析的单测。不依赖 OCR 模型。"""

from __future__ import annotations

from pathlib import Path

import pytest

from dc_worker.ocr_eval import (
    ManifestError,
    RegionCase,
    RegionResult,
    character_error_rate,
    corpus_character_error_rate,
    evaluate,
    load_manifest,
    normalize,
)


class TestCer:
    def test_identical_is_zero(self):
        assert character_error_rate("项目协作群", "项目协作群") == 0.0

    def test_adjacent_swap_costs_two_substitutions(self):
        # “协作”与“作协”是相邻交换，标准 Levenshtein 需要 2 次替换。
        assert character_error_rate("项目协作群", "项目作协群") == pytest.approx(0.4)

    def test_deletion(self):
        assert character_error_rate("abcd", "abd") == pytest.approx(0.25)

    def test_insertion(self):
        assert character_error_rate("abcd", "abxcd") == pytest.approx(0.25)

    def test_empty_expected_with_text_is_total_error(self):
        assert character_error_rate("", "abc") == 1.0

    def test_both_empty_is_zero(self):
        assert character_error_rate("", "") == 0.0

    def test_recognized_empty(self):
        assert character_error_rate("abcd", "") == 1.0

    def test_longer_recognized(self):
        # 真值 4 字、识别 8 字：编辑距离 4 → 1.0。
        assert character_error_rate("abcd", "abcdefgh") == pytest.approx(1.0)


class TestManifest:
    def test_loads_title_and_draft_cases(self, tmp_path: Path):
        (tmp_path / "t.png").write_bytes(b"x")
        (tmp_path / "d.png").write_bytes(b"x")
        manifest = tmp_path / "manifest.json"
        manifest.write_text(
            '{"cases": [{"label": "case1",'
            ' "title": {"image": "t.png", "expected": "项目群"},'
            ' "draft": {"image": "d.png", "expected": "你好"}}]}',
            encoding="utf-8",
        )
        cases = load_manifest(manifest)
        assert cases == [
            RegionCase("case1", "title", tmp_path / "t.png", "项目群"),
            RegionCase("case1", "draft", tmp_path / "d.png", "你好"),
        ]

    def test_missing_role_is_skipped(self, tmp_path: Path):
        (tmp_path / "t.png").write_bytes(b"x")
        manifest = tmp_path / "manifest.json"
        manifest.write_text(
            '{"cases": [{"label": "case1", "title": {"image": "t.png", "expected": "x"}}]}',
            encoding="utf-8",
        )
        cases = load_manifest(manifest)
        assert len(cases) == 1 and cases[0].role == "title"

    @pytest.mark.parametrize(
        "content",
        ["not json", "[]", '{"cases": {}}', '{"cases": [1]}'],
    )
    def test_rejects_invalid_structure(self, tmp_path: Path, content: str):
        manifest = tmp_path / "manifest.json"
        manifest.write_text(content, encoding="utf-8")
        with pytest.raises(ManifestError):
            load_manifest(manifest)

    @pytest.mark.parametrize("image", ["../outside.png", "C:/outside.png", "/outside.png"])
    def test_rejects_escaping_image_path(self, tmp_path: Path, image: str):
        manifest = tmp_path / "manifest.json"
        manifest.write_text(
            '{"cases": [{"label": "case1", '
            f'"title": {{"image": "{image}", "expected": "x"}}}}]}}',
            encoding="utf-8",
        )
        with pytest.raises(ManifestError, match="相对路径|逃逸清单目录"):
            load_manifest(manifest)

    def test_rejects_missing_image(self, tmp_path: Path):
        manifest = tmp_path / "manifest.json"
        manifest.write_text(
            '{"cases": [{"label": "case1", '
            '"title": {"image": "missing.png", "expected": "x"}}]}',
            encoding="utf-8",
        )
        with pytest.raises(ManifestError, match="文件不存在"):
            load_manifest(manifest)


def result(expected: str, distance: int) -> RegionResult:
    expected_chars = len(normalize(expected))
    return RegionResult(
        label="case",
        role="draft",
        expected=expected,
        recognized="",
        exact_match=distance == 0,
        edit_distance=distance,
        expected_chars=expected_chars,
        cer=distance / expected_chars if expected_chars else float(distance > 0),
        latency_ms=1.0,
    )


class TestCorpusCer:
    def test_weights_by_expected_character_count(self):
        assert corpus_character_error_rate([result("a", 1), result("123456789", 0)]) == 0.1

    def test_empty_corpus_is_zero(self):
        assert corpus_character_error_rate([]) == 0.0

    def test_empty_expected_with_recognition_is_error(self):
        assert corpus_character_error_rate([result("", 2)]) == 1.0


class TestNormalize:
    def test_removes_all_whitespace(self):
        assert normalize(" 项目\n协作\t群 ") == "项目协作群"


class FakeEngine:
    def __call__(self, _image: bytes):
        return [[[0, 0, 1, 1], "项目群", 0.99]], [1.0, 2.0]


class TestEvaluate:
    def test_maps_engine_output_and_latency(self, tmp_path: Path, monkeypatch):
        times = iter([10.0, 10.125])
        monkeypatch.setattr("dc_worker.ocr_eval.perf_counter", lambda: next(times))
        image = tmp_path / "title.png"
        image.write_bytes(b"png")
        results = evaluate([RegionCase("case", "title", image, "项目群")], FakeEngine())
        assert len(results) == 1
        assert results[0].recognized == "项目群"
        assert results[0].exact_match
        assert results[0].edit_distance == 0
        assert results[0].latency_ms == 125.0

    def test_blank_image_without_engine_timings(self, tmp_path: Path):
        image = tmp_path / "blank.png"
        image.write_bytes(b"png")
        results = evaluate([RegionCase("blank", "draft", image, "hello")], lambda _: (None, None))
        assert results[0].recognized == ""
        assert results[0].cer == 1.0
        assert results[0].latency_ms >= 0
