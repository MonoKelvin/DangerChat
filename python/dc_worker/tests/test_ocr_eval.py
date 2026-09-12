"""CER 计算与清单解析的单测。不依赖 OCR 模型。"""

from __future__ import annotations

from pathlib import Path

import pytest

from dc_worker.ocr_eval import RegionCase, character_error_rate, load_manifest


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
        manifest = tmp_path / "manifest.json"
        manifest.write_text(
            '{"cases": [{"label": "case1", "title": {"image": "t.png", "expected": "x"}}]}',
            encoding="utf-8",
        )
        cases = load_manifest(manifest)
        assert len(cases) == 1 and cases[0].role == "title"
