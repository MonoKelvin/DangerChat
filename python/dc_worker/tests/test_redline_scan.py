"""红线扫描器的有效性自检。

合规声明的可信度取决于扫描器本身是否有效。一个只会输出「通过」的扫描器
和没有扫描器等价，因此这里同时验证两个方向：

- 植入的违规必须被捕获（不漏报）；
- 合法的自定向调用与注释不得被误判（不误报）。

依据 `docs/合规与风险说明.md` §4.2 与 `docs/03_危信v1.0_工程测试与上线手册.md` §11。
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SCANNER = REPO_ROOT / "scripts" / "redline_scan.py"

sys.path.insert(0, str(REPO_ROOT / "scripts"))

from redline_scan import is_reference_only, scan  # noqa: E402


def write_source(root: Path, body: str, name: str = "probe.rs") -> None:
    """在临时扫描根目录下写一个源文件。"""
    (root / "crates").mkdir(parents=True, exist_ok=True)
    (root / "crates" / name).write_text(body, encoding="utf-8")


@pytest.fixture
def scan_root(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """把扫描根目录指向临时目录，避免依赖仓库真实内容。"""
    import redline_scan

    monkeypatch.setattr(redline_scan, "REPO_ROOT", tmp_path)
    return tmp_path


class TestDetectsViolations:
    """植入违规必须被捕获。任何一项静默通过都意味着合规声明失效。"""

    @pytest.mark.parametrize(
        ("snippet", "category"),
        [
            ("unsafe { SendInput(1, &inputs, size); }", "INPUT_INJECTION"),
            ("keybd_event(0x0D, 0, 0, 0);", "INPUT_INJECTION"),
            ("mouse_event(flags, 0, 0, 0, 0);", "INPUT_INJECTION"),
            ("PostMessageW(target, WM_KEYDOWN, w, l);", "INPUT_INJECTION"),
            ("SendMessageW(hwnd, WM_SETTEXT, w, l);", "INPUT_INJECTION"),
            ("SendNotifyMessageW(hwnd, msg, w, l);", "INPUT_INJECTION"),
            ("SendMessageTimeoutW(hwnd, msg, w, l, f, t, r);", "INPUT_INJECTION"),
            ("ReadProcessMemory(handle, addr, buf, len, None);", "PROCESS_ACCESS"),
            ("WriteProcessMemory(handle, addr, buf, len, None);", "PROCESS_ACCESS"),
            ("CreateRemoteThread(h, None, 0, f, None, 0, None);", "PROCESS_ACCESS"),
            ("VirtualAllocEx(handle, None, size, kind, prot);", "PROCESS_ACCESS"),
            ("EnumProcessModules(handle, out, size, needed);", "PROCESS_ACCESS"),
            ("PrintWindow(hwnd, hdc, 0);", "EXCLUDED_CAPTURE"),
            ("let ui: IUIAutomation = create()?;", "EXCLUDED_CAPTURE"),
            ("GraphicsCaptureItem::CreateForWindow(hwnd)?;", "EXCLUDED_CAPTURE"),
            (r'let p = "C:\\Program Files\\WeChat\\WeChat.exe";', "TARGET_PATH"),
            ("let db = base.join(\"MicroMsg\");", "TARGET_PATH"),
        ],
    )
    def test_planted_violation_is_caught(self, scan_root: Path, snippet: str, category: str):
        write_source(scan_root, f"fn probe() {{\n    {snippet}\n}}\n")
        findings, scanned = scan(["crates"])
        assert scanned == 1
        assert any(category in f for f in findings), f"未捕获: {snippet}"

    def test_non_keyboard_ll_hook_is_rejected(self, scan_root: Path):
        write_source(
            scan_root,
            "fn probe() {\n    SetWindowsHookExW(WH_GETMESSAGE, Some(p), None, 0);\n}\n",
        )
        findings, _ = scan(["crates"])
        assert any("PROCESS_ACCESS" in f for f in findings)

    def test_keyboard_ll_hook_is_allowed(self, scan_root: Path):
        write_source(
            scan_root,
            "fn probe() {\n    SetWindowsHookExW(WH_KEYBOARD_LL, Some(p), None, 0);\n}\n",
        )
        findings, _ = scan(["crates"])
        assert findings == []


class TestCommentHandlingCannotHideCode:
    """注释判定过宽会制造可绕过的盲区，这里锁住修复后的行为。"""

    @pytest.mark.parametrize(
        "line",
        [
            # Rust 解引用与裸指针类型同样以 `*` 开头。
            "*mut handle = SendInput(evt);",
            # 行内出现三引号不代表整行是文档字符串。
            's = """doc"""; SendInput(evt)',
            # 以 r" 开头的行仍可能在后面接真实调用。
            'r"lit"; SendInput(evt)',
        ],
    )
    def test_code_disguised_as_comment_is_still_scanned(self, scan_root: Path, line: str):
        write_source(scan_root, f"{line}\n")
        findings, _ = scan(["crates"])
        assert findings, f"被当作注释而漏检: {line}"

    @pytest.mark.parametrize(
        "line",
        [
            "// SendInput 在此禁止",
            "/// 文档：不得调用 SendInput",
            "//! 模块级说明：SendInput 属于红线",
            "# Python 注释：SendInput",
            " * 块注释续行提到 SendInput",
        ],
    )
    def test_real_comments_are_skipped(self, line: str):
        assert is_reference_only(line)

    def test_blank_lines_are_skipped(self):
        assert is_reference_only("")
        assert is_reference_only("   ")

    def test_deref_is_not_a_comment(self):
        assert not is_reference_only("*mut u8")
        assert not is_reference_only("*ptr = value;")

    def test_identifier_containing_api_name_is_not_a_violation(self, scan_root: Path):
        """词边界限定：`SendInput_wrapper` 是自有标识符，不是对该 API 的调用。

        放宽为子串匹配会让扫描器对无关命名大量误报，反而促使开发者忽略它。
        """
        write_source(scan_root, "fn probe() {\n    let my_SendInput_wrapper = 1;\n}\n")
        findings, _ = scan(["crates"])
        assert findings == []


class TestSelfDirectedCallsAreAllowed:
    """PostThreadMessage 面向本进程线程，不是外部窗口，不得误报。"""

    def test_post_thread_message_passes(self, scan_root: Path):
        write_source(
            scan_root,
            "fn wake() {\n    PostThreadMessageW(tid, WM_APP, WPARAM(0), LPARAM(0));\n}\n",
        )
        findings, _ = scan(["crates"])
        assert findings == []

    def test_post_thread_message_does_not_mask_post_message(self, scan_root: Path):
        """同一行里的 PostMessage 不能因为 PostThreadMessage 的存在而被放过。"""
        write_source(
            scan_root,
            "fn probe() {\n"
            "    PostThreadMessageW(tid, m, w, l); PostMessageW(other, m, w, l);\n"
            "}\n",
        )
        findings, _ = scan(["crates"])
        assert any("INPUT_INJECTION" in f for f in findings)


class TestScanCoverage:
    def test_skips_build_output_directories(self, scan_root: Path):
        target = scan_root / "crates" / "target"
        target.mkdir(parents=True)
        (target / "generated.rs").write_text("SendInput(a);", encoding="utf-8")
        findings, scanned = scan(["crates"])
        assert scanned == 0
        assert findings == []

    def test_ignores_non_source_suffixes(self, scan_root: Path):
        (scan_root / "crates").mkdir(parents=True)
        (scan_root / "crates" / "notes.md").write_text("SendInput(a);", encoding="utf-8")
        findings, scanned = scan(["crates"])
        assert scanned == 0
        assert findings == []

    def test_missing_root_is_not_an_error(self, scan_root: Path):
        findings, scanned = scan(["does_not_exist"])
        assert scanned == 0
        assert findings == []

    def test_exclusion_list_covers_only_this_test_file(self):
        """排除项是绕过合规检查的潜在入口，必须保持最小且精确。

        任何新增排除都应在此显式登记并说明理由，否则视为回归。
        """
        import redline_scan

        assert redline_scan.EXCLUDED_PATHS == {
            Path("python/dc_worker/tests/test_redline_scan.py")
        }

    def test_excluded_file_would_otherwise_be_flagged(self, scan_root: Path):
        """确认排除的不是一个「本来就干净」的文件——否则排除项形同虚设，
        也说明测试没有真正包含违规样本。
        """
        src = REPO_ROOT / "python" / "dc_worker" / "tests" / "test_redline_scan.py"
        target = scan_root / "crates" / "copy_of_selftest.py"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(src.read_text(encoding="utf-8"), encoding="utf-8")
        findings, _ = scan(["crates"])
        assert findings, "自检文件不含违规样本，说明它没有真正测试捕获能力"


class TestExitCode:
    """CI 依赖退出码阻断合并，必须可靠。

    扫描器按自身文件位置的父目录推断仓库根（`REPO_ROOT`），
    因此把它复制到 `<tmp>/scripts/` 下即可让它扫描 `<tmp>`。
    """

    @staticmethod
    def run_scanner_on(tree: Path) -> subprocess.CompletedProcess:
        scripts = tree / "scripts"
        scripts.mkdir(parents=True, exist_ok=True)
        copy = scripts / "redline_scan.py"
        copy.write_text(SCANNER.read_text(encoding="utf-8"), encoding="utf-8")
        return subprocess.run([sys.executable, str(copy)], capture_output=True)

    def test_clean_tree_exits_zero(self, tmp_path: Path):
        crates = tmp_path / "crates"
        crates.mkdir()
        (crates / "ok.rs").write_text(
            "fn wake() { PostThreadMessageW(tid, m, w, l); }", encoding="utf-8"
        )
        assert self.run_scanner_on(tmp_path).returncode == 0

    def test_violation_exits_nonzero(self, tmp_path: Path):
        crates = tmp_path / "crates"
        crates.mkdir()
        (crates / "bad.rs").write_text("fn f() { SendInput(1, p, s); }", encoding="utf-8")
        assert self.run_scanner_on(tmp_path).returncode == 1

    def test_real_repository_is_clean(self):
        """仓库当前状态必须通过扫描——这是合规声明的事实基础。"""
        result = subprocess.run(
            [sys.executable, str(SCANNER)],
            capture_output=True,
            cwd=REPO_ROOT,
        )
        assert result.returncode == 0
