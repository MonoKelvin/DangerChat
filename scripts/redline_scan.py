#!/usr/bin/env python
"""红线静态扫描。

依据 `docs/合规与风险说明.md` §4.2、§6 与 `docs/03_危信v1.0_工程测试与上线手册.md` §11。

在生产源码中出现以下任一情况即失败：
- 输入注入 API：SendInput / keybd_event / mouse_event
- 目标进程访问：ReadProcessMemory / WriteProcessMemory / CreateRemoteThread / VirtualAllocEx
- 向外部窗口投递消息：PostMessage / SendMessage / SendNotifyMessage / SendMessageTimeout
- 被明确排除的采集方式：PrintWindow / UI Automation / 窗口定向捕获
- 指向目标程序安装目录或数据目录的路径常量

当前不设任何白名单：危信不实现 `SendInput` 自进程存活探针，
因此“不调用任何输入注入 API”是可被静态验证的绝对断言。

用法：
    python scripts/redline_scan.py            # 扫描生产源码
    python scripts/redline_scan.py --verbose  # 同时打印已扫描文件数
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# 生产源码根目录。测试与尖峰的 Harness 目录单独处理。
PRODUCTION_ROOTS = ["crates", "apps", "python", "packages"]

# M0 尖峰同样受红线约束：它演示的正是“只做减法”的实现。
SPIKE_ROOTS = ["spikes"]

SOURCE_SUFFIXES = {".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".toml", ".json"}

SKIP_DIRS = {
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    "dist",
    ".git",
    ".ruff_cache",
    ".pytest_cache",
}

RULES: list[tuple[str, str, str]] = [
    ("INPUT_INJECTION", r"\bSendInput\b", "禁止向系统或目标程序注入输入"),
    ("INPUT_INJECTION", r"\bkeybd_event\b", "禁止合成键盘事件"),
    ("INPUT_INJECTION", r"\bmouse_event\b", "禁止合成鼠标事件"),
    ("INPUT_INJECTION", r"\bSendMessageTimeout[AW]?\b", "禁止向外部窗口投递消息"),
    ("INPUT_INJECTION", r"\bSendNotifyMessage[AW]?\b", "禁止向外部窗口投递消息"),
    ("INPUT_INJECTION", r"\bPostMessage[AW]?\b", "禁止向外部窗口投递消息"),
    ("INPUT_INJECTION", r"\bSendMessage[AW]?\b", "禁止向外部窗口投递消息"),
    ("PROCESS_ACCESS", r"\bReadProcessMemory\b", "禁止读取目标进程内存"),
    ("PROCESS_ACCESS", r"\bWriteProcessMemory\b", "禁止写入目标进程内存"),
    ("PROCESS_ACCESS", r"\bCreateRemoteThread\b", "禁止远程线程注入"),
    ("PROCESS_ACCESS", r"\bVirtualAllocEx\b", "禁止在目标进程分配内存"),
    ("PROCESS_ACCESS", r"\bEnumProcessModules\b", "禁止枚举目标进程模块"),
    ("PROCESS_ACCESS", r"\bSetWindowsHookEx[AW]?\s*\(\s*WH_(?!KEYBOARD_LL)", "只允许 WH_KEYBOARD_LL"),
    ("EXCLUDED_CAPTURE", r"\bPrintWindow\b", "禁止要求目标窗口自行渲染"),
    ("EXCLUDED_CAPTURE", r"\bUIAutomation\b", "禁止读取目标控件树"),
    ("EXCLUDED_CAPTURE", r"\bIUIAutomation\b", "禁止读取目标控件树"),
    ("EXCLUDED_CAPTURE", r"CreateForWindow", "禁止窗口定向捕获，只允许显示器级捕获"),
    ("TARGET_PATH", r"(?i)\\WeChat\\|/WeChat/|WeChat\.exe|MicroMsg", "禁止引用目标程序目录或数据"),
]

# `PostThreadMessage` 只面向本进程自己的线程，不是外部窗口，明确允许。
ALLOWED_SELF_DIRECTED = re.compile(r"\bPostThreadMessage[AW]?\b")

COMMENT_PREFIXES = ("//", "#", "///", "//!", "*")


def is_reference_only(line: str) -> bool:
    """注释、文档字符串与扫描规则自身不算违规。"""
    stripped = line.strip()
    if stripped.startswith(COMMENT_PREFIXES):
        return True
    if '"""' in stripped or stripped.startswith('r"'):
        return True
    return False


def iter_sources(roots: list[str]):
    for root in roots:
        base = REPO_ROOT / root
        if not base.exists():
            continue
        for path in base.rglob("*"):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            yield path


def scan(roots: list[str]) -> tuple[list[str], int]:
    findings: list[str] = []
    scanned = 0
    for path in iter_sources(roots):
        scanned += 1
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError) as exc:
            findings.append(f"{path}: 无法读取 ({exc})")
            continue
        for lineno, line in enumerate(text.splitlines(), start=1):
            if is_reference_only(line):
                continue
            without_allowed = ALLOWED_SELF_DIRECTED.sub("", line)
            for category, pattern, reason in RULES:
                if re.search(pattern, without_allowed):
                    rel = path.relative_to(REPO_ROOT)
                    findings.append(f"{rel}:{lineno}: [{category}] {reason} -> {line.strip()}")
    return findings, scanned


def main() -> int:
    verbose = "--verbose" in sys.argv
    all_roots = PRODUCTION_ROOTS + SPIKE_ROOTS
    findings, scanned = scan(all_roots)

    if verbose:
        print(f"已扫描 {scanned} 个源文件，覆盖目录：{', '.join(all_roots)}")

    if findings:
        print("红线扫描失败：")
        for item in findings:
            print(f"  {item}")
        print()
        print("任何触碰请先阅读 docs/合规与风险说明.md，未经评审不得绕过。")
        return 1

    print(f"红线扫描通过（{scanned} 个源文件）。")
    print("未发现输入注入、目标进程访问、排除的采集方式或目标路径常量。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
