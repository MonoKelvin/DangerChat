//! 危信 CI 红线静态扫描（合规 §6、AC-04）。
//!
//! **规则：零引用、零白名单例外。** 扫描范围内出现任何一个禁用 API 标识符或微信数据目录
//! 常量，即判定为违规并非零退出。
//!
//! ## 为什么不给自己留白名单
//!
//! 扫描器自身也在扫描范围内。为了让「无例外」真正成立，本文件的禁用标识符全部由**片段拼接**
//! 得到（例如 `concat!("Send", "Input")`），源码中不出现完整标识符，因此无需排除自身。
//! 同理，代码注释里也不得出现完整标识符——想引用它们请指向《合规与风险说明》§4.2 的清单。
//!
//! ## 扫描范围
//!
//! 只扫**可执行代码与配置**（见 [`CODE_EXTENSIONS`]）；`docs/` 下的说明文档是散文，
//! 需要原样引用协议与 API 名称（合规文档 §2 就是逐字摘录），不纳入扫描。
//! 跳过 `target/`、`node_modules/`、`.git/` 等构建与依赖目录。

use std::fs;
use std::path::{Path, PathBuf};

/// 被判为违规的项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: PathBuf,
    /// 1-based 行号
    pub line: usize,
    pub needle: String,
    pub category: &'static str,
    /// 去首尾空白的原始行
    pub text: String,
}

/// 扫描结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub violations: Vec<Violation>,
    pub files_scanned: usize,
    pub files_skipped: usize,
}

impl ScanReport {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// 纳入扫描的代码/配置扩展名。
pub const CODE_EXTENSIONS: &[&str] = &[
    "rs", "c", "cc", "cpp", "h", "hpp", "cs", "go", "py", "java", "kt", "swift", "ts", "tsx", "js",
    "jsx", "mjs", "cjs", "vue", "svelte", "html", "css", "scss", "toml", "json", "yaml", "yml",
    "ps1", "bat", "cmd", "sh", "sql",
];

/// 永不进入的目录（构建产物、依赖、缓存、文档、二进制资源）。
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "artifacts",
    "reports",
    ".workbuddy",
    ".claude",
    "docs",
    "resources",
    ".idea",
    ".vscode",
    "vendor",
];

/// 单个文件扫描上限，超过视为生成物/压缩产物。
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// 禁用标识符清单。**必须由片段拼接**，否则扫描器会命中自己。
pub fn needles() -> Vec<(&'static str, String)> {
    // 类别常量
    const INJECT: &str = "输入注入";
    const MEM: &str = "内存/线程操纵";
    const CAPTURE: &str = "窗口定向捕获";
    const WECHAT_PATH: &str = "微信数据路径";

    let mut out: Vec<(&'static str, String)> = Vec::new();

    // 输入注入：向目标程序投递按键/鼠标/窗口消息（合规红线 3、C-08）
    for needle in [
        concat!("Send", "Input"),
        concat!("keybd", "_event"),
        concat!("mouse", "_event"),
        concat!("Send", "Message"),
        concat!("Post", "Message"),
        concat!("SendNotify", "Message"),
    ] {
        out.push((INJECT, needle.to_string()));
    }

    // 内存/线程操纵：注入 DLL、读写目标进程内存（永久排除，合规红线 1）
    for needle in [
        concat!("WriteProcess", "Memory"),
        concat!("ReadProcess", "Memory"),
        concat!("CreateRemote", "Thread"),
        concat!("VirtualAlloc", "Ex"),
        concat!("NtRead", "Virtual", "Memory"),
        concat!("NtWrite", "Virtual", "Memory"),
    ] {
        out.push((MEM, needle.to_string()));
    }

    // 窗口定向捕获：会向目标窗口发消息或画捕获边框（§4.1）
    for needle in [concat!("Print", "Window"), concat!("Get", "WindowDC")] {
        out.push((CAPTURE, needle.to_string()));
    }

    // 微信数据目录常量（红线 2：不读微信文件）。单双反斜杠两种写法都要拦。
    for needle in [
        concat!("WeChat", " Files"),
        concat!("WeChat", "Files"),
        concat!("Msg", "Attach"),
        concat!("File", "Storage"),
        concat!("Tencent", "\\WeChat"),
        concat!("Tencent", "\\\\WeChat"),
        concat!("%APPDATA%", "\\Tencent"),
        concat!("%APPDATA%", "\\\\Tencent"),
    ] {
        out.push((WECHAT_PATH, needle.to_string()));
    }

    out
}

/// 判断扩展名是否纳入扫描。
pub fn is_code_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => CODE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// 扫描仓库根目录。
pub fn scan(root: &Path) -> ScanReport {
    let needles = needles();
    let mut report = ScanReport::default();
    walk(root, &needles, &mut report);
    report.violations.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.needle.cmp(&b.needle))
    });
    report
}

fn walk(dir: &Path, needles: &[(&'static str, String)], report: &mut ScanReport) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for path in children {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&path, needles, report);
            continue;
        }
        if !is_code_file(&path) {
            report.files_skipped += 1;
            continue;
        }
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
            report.files_skipped += 1;
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            // 非 UTF-8（二进制/生成物）跳过
            report.files_skipped += 1;
            continue;
        };
        report.files_scanned += 1;
        scan_text(&path, &text, needles, report);
    }
}

/// 对单份文本做逐行匹配（拆出来便于自测）。
pub fn scan_text(
    path: &Path,
    text: &str,
    needles: &[(&'static str, String)],
    report: &mut ScanReport,
) {
    for (idx, line) in text.lines().enumerate() {
        for (category, needle) in needles {
            if line.contains(needle.as_str()) {
                report.violations.push(Violation {
                    path: path.to_path_buf(),
                    line: idx + 1,
                    needle: needle.clone(),
                    category,
                    text: line.trim().to_string(),
                });
            }
        }
    }
}
