//! 红线扫描 CLI。
//!
//! ```text
//! redline_scan [--root <dir>] [--json] [--quiet]
//!
//! 退出码：0 = 零引用（通过）；1 = 发现违规；2 = 用法/IO 错误
//! ```
//!
//! CI 用法（AC-04）：`cargo run -p redline_scan -- --root .`

use std::path::PathBuf;
use std::process::ExitCode;

use redline_scan::{scan, CODE_EXTENSIONS, SKIP_DIRS};

fn main() -> ExitCode {
    let mut root = PathBuf::from(".");
    let mut json = false;
    let mut quiet = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => match args.next() {
                Some(v) => root = PathBuf::from(v),
                None => {
                    eprintln!("--root 需要一个目录参数");
                    return ExitCode::from(2);
                }
            },
            "--json" => json = true,
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数：{other}");
                print_help();
                return ExitCode::from(2);
            }
        }
    }

    if !root.is_dir() {
        eprintln!("扫描根目录不存在：{}", root.display());
        return ExitCode::from(2);
    }

    let report = scan(&root);

    if json {
        println!("{}", to_json(&report));
    } else if !quiet {
        print_report(&root, &report);
    }

    if report.is_clean() {
        if !json && !quiet {
            println!("✅ 红线扫描通过：零引用（AC-04）");
        }
        ExitCode::SUCCESS
    } else {
        if !json && !quiet {
            println!("❌ 红线扫描失败：{} 处违规", report.violations.len());
        }
        ExitCode::from(1)
    }
}

fn print_help() {
    println!("危信红线扫描（合规 §6 / AC-04）");
    println!();
    println!("用法：redline_scan [--root <dir>] [--json] [--quiet]");
    println!();
    println!("扫描扩展名：{}", CODE_EXTENSIONS.join(", "));
    println!("跳过目录：{}", SKIP_DIRS.join(", "));
    println!("说明：docs/ 下的说明文档需逐字引用协议与 API 名称，不纳入扫描。");
}

fn print_report(root: &std::path::Path, report: &redline_scan::ScanReport) {
    println!("红线扫描根目录：{}", root.display());
    println!(
        "扫描 {} 个文件，跳过 {} 个（非代码/超限/非 UTF-8）",
        report.files_scanned, report.files_skipped
    );
    if report.is_clean() {
        return;
    }
    println!();
    for v in &report.violations {
        println!(
            "{}:{}  [{}] 命中 `{}`",
            v.path.display(),
            v.line,
            v.category,
            v.needle
        );
        println!("    {}", v.text);
    }
}

fn to_json(report: &redline_scan::ScanReport) -> String {
    // 手写最小 JSON：避免为一个 CI 小工具引入 serde 依赖
    let mut items = Vec::new();
    for v in &report.violations {
        items.push(format!(
            "{{\"path\":{},\"line\":{},\"category\":{},\"needle\":{},\"text\":{}}}",
            json_str(&v.path.to_string_lossy()),
            v.line,
            json_str(v.category),
            json_str(&v.needle),
            json_str(&v.text)
        ));
    }
    format!(
        "{{\"clean\":{},\"filesScanned\":{},\"filesSkipped\":{},\"violations\":[{}]}}",
        report.is_clean(),
        report.files_scanned,
        report.files_skipped,
        items.join(",")
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
