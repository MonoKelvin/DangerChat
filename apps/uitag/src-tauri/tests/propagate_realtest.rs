//! 真实截图传播性能冒烟（手动 `cargo test -p uitag --test propagate_realtest -- --ignored --nocapture`）：
//! 取两张真实微信截图，模拟四框标注，测单框耗时与匹配率。

#[test]
#[ignore]
fn prop_real_screenshots_perf() {
    use uitag_lib::propagate::*;
    use uitag_lib::state::AnnoBox;

    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../resources/training_set/wechat_snapcaptures"
    );
    let dir = std::path::Path::new(dir);
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    if files.len() < 2 {
        eprintln!("无真实截图，跳过");
        return;
    }
    let src = files[0].to_string_lossy().into_owned();
    let targets: Vec<String> = files[1..6]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    // 模拟典型四框（聊天列表左侧栏、输入框底部、对象名顶部、整窗口）
    let boxes = vec![
        AnnoBox { tag: "chat_list".into(), x: 0.0, y: 100.0, w: 250.0, h: 600.0 },
        AnnoBox { tag: "chat_window".into(), x: 250.0, y: 0.0, w: 700.0, h: 700.0 },
        AnnoBox { tag: "chat_target".into(), x: 300.0, y: 10.0, w: 300.0, h: 60.0 },
        AnnoBox { tag: "msg_input".into(), x: 260.0, y: 620.0, w: 680.0, h: 80.0 },
    ];

    let req = PropagateRequest {
        src_path: src,
        boxes,
        targets,
        min_confidence: 0.6,
        allow_rescale: true,
    };
    let t0 = std::time::Instant::now();
    let results = propagate(&req).unwrap();
    let dt = t0.elapsed();
    eprintln!("5 张图 × 4 框 = {:.2}s（{:.0}ms/框）", dt.as_secs_f64(), dt.as_millis() as f64 / 20.0);
    for r in &results {
        let file = r.path.rsplit(['\\', '/']).next().unwrap_or(&r.path);
        let summary: Vec<String> = r
            .boxes
            .iter()
            .map(|b| format!("{}:{:.2}", b.box_.tag, b.confidence))
            .collect();
        eprintln!("  {file}: {} 框 [{}]", r.boxes.len(), summary.join(" "));
    }
}
