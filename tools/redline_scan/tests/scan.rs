//! 红线扫描器自测（AC-04）。
//!
//! 注意：本文件同样处于扫描范围内，因此**测试数据也不得出现完整禁用标识符**，
//! 一律用片段拼接（`format!("{}{}", "Send", "Input")`）。

use std::fs;
use std::path::{Path, PathBuf};

use redline_scan::{needles, scan};

fn write(root: &Path, rel: &str, content: &str) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// 代码文件中的禁用标识符必须被命中，并给出准确位置与类别
#[test]
fn flags_needles_in_code_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let inject = format!("{}{}", "Send", "Input");
    let path = write(
        dir.path(),
        "src/inject.rs",
        &format!("fn a() {{}}\n// 正常注释\nlet api = \"{inject}\";\nfn b() {{}}\n"),
    );

    let report = scan(dir.path());
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    let v = &report.violations[0];
    assert_eq!(v.path, path);
    assert_eq!(v.line, 3);
    assert_eq!(v.needle, inject);
    assert_eq!(v.category, "输入注入");
    assert!(v.text.contains(&inject));
    assert!(!report.is_clean());
    assert_eq!(report.files_scanned, 1);
}

/// 各类型禁用项都能被识别（覆盖四类清单）
#[test]
fn flags_all_categories() {
    let dir = tempfile::TempDir::new().unwrap();
    let cases = [
        (format!("{}{}", "keybd", "_event"), "输入注入"),
        (format!("{}{}", "mouse", "_event"), "输入注入"),
        (format!("{}{}", "Post", "Message"), "输入注入"),
        (format!("{}{}", "WriteProcess", "Memory"), "内存/线程操纵"),
        (format!("{}{}", "CreateRemote", "Thread"), "内存/线程操纵"),
        (format!("{}{}", "Print", "Window"), "窗口定向捕获"),
        (format!("{}{}", "WeChat", " Files"), "微信数据路径"),
        (format!("{}{}", "Msg", "Attach"), "微信数据路径"),
    ];
    let mut body = String::new();
    for (needle, _) in &cases {
        body.push_str(&format!("const X: &str = \"{needle}\";\n"));
    }
    write(dir.path(), "src/bad.rs", &body);

    let report = scan(dir.path());
    assert_eq!(
        report.violations.len(),
        cases.len(),
        "{:?}",
        report.violations
    );
    for (needle, category) in &cases {
        assert!(
            report
                .violations
                .iter()
                .any(|v| &v.needle == needle && v.category == *category),
            "缺少 {needle} / {category}"
        );
    }
}

/// 文档、构建产物、依赖目录、非代码文件均不纳入扫描
#[test]
fn ignores_prose_and_skipped_dirs() {
    let dir = tempfile::TempDir::new().unwrap();
    let inject = format!("{}{}", "Send", "Input");
    write(dir.path(), "docs/redline.md", &inject);
    write(dir.path(), "target/debug/x.rs", &inject);
    write(dir.path(), "node_modules/pkg/x.js", &inject);
    write(dir.path(), "resources/data.json", &inject);
    write(dir.path(), ".workbuddy/memory.md", &inject);
    write(dir.path(), "notes.txt", &inject);

    let report = scan(dir.path());
    assert!(report.is_clean(), "{:?}", report.violations);
    assert_eq!(report.files_scanned, 0, "没有任何代码文件被扫描");
    // 只有 notes.txt 是「非代码扩展名」被计数；docs/target/node_modules/.workbuddy/resources
    // 是整目录跳过，连遍历都不进入
    assert_eq!(report.files_skipped, 1);
}

/// 二进制/非 UTF-8 文件与超大文件被跳过而不是崩溃
#[test]
fn skips_binary_and_oversize_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let inject = format!("{}{}", "Send", "Input");
    // 非 UTF-8：含非法字节序列
    let mut bytes = inject.into_bytes();
    bytes.extend_from_slice(&[0xFF, 0xFE, 0xFD]);
    fs::write(dir.path().join("binary.rs"), &bytes).unwrap();
    // 超过 2MB
    let big = format!("// pad\n{}", "x".repeat(2 * 1024 * 1024 + 1));
    fs::write(dir.path().join("big.rs"), big).unwrap();
    // 合法小文件作为对照
    write(dir.path(), "ok.rs", "fn main() {}\n");

    let report = scan(dir.path());
    assert!(report.is_clean());
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.files_skipped, 2);
}

/// 扫描器自身源码不得出现完整禁用标识符（片段拼接纪律的自检）
#[test]
fn scanner_source_avoids_literal_needles() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut text = String::new();
    for entry in fs::read_dir(&src).unwrap().flatten() {
        text.push_str(&fs::read_to_string(entry.path()).unwrap());
    }
    for (category, needle) in needles() {
        assert!(
            !text.contains(&needle),
            "红线扫描器源码中出现了完整标识符 `{needle}`（{category}），应改为片段拼接"
        );
    }
}

/// 仓库本体零引用（AC-04 的实际门禁）
#[test]
fn repository_is_redline_clean() {
    let root = repo_root();
    let report = scan(&root);
    assert!(
        report.is_clean(),
        "仓库存在红线引用：{:#?}",
        report.violations
    );
    assert!(report.files_scanned > 5, "应当扫到仓库的源码文件");
}
