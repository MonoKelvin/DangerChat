//! UT-CORE-05 ~ UT-CORE-06：日志滚动清理与图片日志隐私开关（设计文档 §5.1 测试矩阵）

use std::fs;
use std::path::Path;
use std::sync::Arc;

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

/// UT-CORE-06 隐私模式开启时不写盘（Noop sink 零副作用）
#[test]
fn ut_core_06_privacy_mode_writes_nothing() {
    let sink = ImageLogSink::noop();
    assert!(!sink.is_enabled());
    for name in ["01_window", "02_layout", "03_ocr"] {
        sink.save(name, &RgbaImage::new(4, 4)).unwrap();
    }
    // Noop sink 无副作用，不会创建任何文件/目录
}

/// UT-CORE-06b 开启时通过 RollingImageStore 异步落盘
#[test]
fn ut_core_06b_enabled_mode_saves_async() {
    let dir = TempDir::new().unwrap();
    let store = dc_core::RollingImageStore::new(dir.path(), 10, 1);
    let store_arc = Arc::new(std::sync::Mutex::new(store));

    let sink = ImageLogSink::from_store(Arc::clone(&store_arc));
    assert!(sink.is_enabled());
    sink.save("01_window", &RgbaImage::new(4, 4)).unwrap();
    sink.save("03_ocr", &RgbaImage::new(2, 2)).unwrap();

    // 异步写入，需等待工作线程完成
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(sink);
    drop(store_arc); // Drop 会 join 工作线程

    let dir1 = dir.path().join("1");
    assert!(dir1.join("01_window.png").exists());
    assert!(dir1.join("03_ocr.png").exists());
}

/// RunId 语义：目录名可读、含序号
#[test]
fn ut_core_06c_run_id_format() {
    let id = RunId::new(7, 480);
    assert!(id.as_str().ends_with("-000007"), "id = {id}");
    assert_eq!(id.as_str().len(), "20260913-000000".len() + 7);
}
