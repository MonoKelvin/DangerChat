//! UT-CORE-05 ~ UT-CORE-06：日志滚动清理与图片日志隐私开关（设计文档 §5.1 测试矩阵）

use std::fs;
use std::path::Path;

use dc_core::logging::prune_logs;
use dc_core::time;
use dc_core::{ImageLogSink, RunId};
use image::RgbaImage;
use tempfile::TempDir;

fn write_log(dir: &Path, days_ago: i64, bytes: usize) -> std::path::PathBuf {
    let date = time::date_string(time::unix_now() - days_ago * 86_400, 0);
    let path = dir.join(format!("danger-{date}.log"));
    fs::write(&path, vec![b'x'; bytes]).unwrap();
    path
}

/// UT-CORE-05 日志滚动：超保留天数 / 超总量均被清理
#[test]
fn ut_core_05_prune_by_age_and_size() {
    // ---- 保留天数 ----
    let dir = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for days_ago in 0..=9 {
        paths.push(write_log(dir.path(), days_ago, 16));
    }
    let report = prune_logs(dir.path(), "danger", 7, 200, 0).unwrap();
    assert_eq!(report.removed.len(), 2, "应删除 today-8 / today-9");
    assert_eq!(report.remaining_files, 8, "today 到 today-7 共 8 个");
    assert!(paths[0].exists(), "今天的日志永不删除（进程持有写句柄）");
    assert!(!paths[8].exists() && !paths[9].exists());
    assert!(report.freed_bytes > 0);

    // 保留天数内的文件不受影响
    for path in paths.iter().take(8) {
        assert!(path.exists(), "{} 不应被删", path.display());
    }

    // ---- 总量上限（从最旧删起） ----
    let dir2 = TempDir::new().unwrap();
    for days_ago in [1, 2, 3] {
        write_log(dir2.path(), days_ago, 700 * 1024);
    }
    write_log(dir2.path(), 0, 10);
    // 3 × 700KB = 2100KB > 1MB：删最旧的 today-3、today-2 后 700KB ≤ 1MB
    let report2 = prune_logs(dir2.path(), "danger", 7, 1, 0).unwrap();
    assert_eq!(report2.removed.len(), 2, "removed = {:?}", report2.removed);
    assert_eq!(report2.remaining_files, 2);
    assert!(report2.remaining_bytes < 1024 * 1024);

    // 非日志文件不受清理影响
    let dir3 = TempDir::new().unwrap();
    fs::write(dir3.path().join("keep-me.txt"), b"hello").unwrap();
    write_log(dir3.path(), 30, 16);
    prune_logs(dir3.path(), "danger", 7, 200, 0).unwrap();
    assert!(dir3.path().join("keep-me.txt").exists());

    // 目录不存在时不报错（首次运行）
    let missing = TempDir::new().unwrap().path().join("nope");
    assert_eq!(
        prune_logs(&missing, "danger", 7, 200, 0)
            .unwrap()
            .removed
            .len(),
        0
    );
}

/// UT-CORE-06 隐私模式开启时 runs/ 目录零文件（断言磁盘为空）
#[test]
fn ut_core_06_privacy_mode_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let runs = dir.path().join("runs");

    // 关闭图片日志（隐私模式 / privacy.no_image_logs = true）
    let sink = ImageLogSink::new(&runs, RunId::from_raw("20260913-000000-000001"), false);
    assert!(!sink.is_enabled());
    assert!(sink.run_dir().is_none());
    for name in ["01_window", "02_layout", "03_ocr"] {
        sink.save(name, &RgbaImage::new(4, 4)).unwrap();
    }
    assert!(!runs.exists(), "隐私模式下不得创建任何目录/文件");

    // 反向对照：开启时确实落盘，证明上一条不是空断言
    let sink_on = ImageLogSink::new(&runs, RunId::from_raw("20260913-000000-000002"), true);
    assert!(sink_on.is_enabled());
    sink_on.save("01_window", &RgbaImage::new(4, 4)).unwrap();
    sink_on.save("03_ocr", &RgbaImage::new(2, 2)).unwrap();
    let run_dir = sink_on.run_dir().unwrap();
    assert!(run_dir.join("01_window.png").exists());
    assert!(run_dir.join("03_ocr.png").exists());
    let count = fs::read_dir(&run_dir).unwrap().count();
    assert_eq!(count, 2, "一个 run 目录只放本轮的图");

    // RunId 语义：目录名可读、含序号
    let id = RunId::new(7, 480);
    assert!(id.as_str().ends_with("-000007"), "id = {id}");
    assert_eq!(id.as_str().len(), "20260913-000000".len() + 7);
}
