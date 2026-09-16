//! 离线交叉验证：以数据集中一张人工标注图为模板，传播到其余图，与真值对比。
//! 用法：cargo run --example xval_propagate -- <dataset_dir> [pairs]
//! dataset_dir 下需有 annotations.json（key = 文件名）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uitag_lib::propagate::{propagate, PropagateRequest};
use uitag_lib::state::{AnnoBox, Autosave};

fn iou(a: &AnnoBox, b: &AnnoBox) -> f64 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let uni = a.w * a.h + b.w * b.h - inter;
    if uni <= 0.0 {
        0.0
    } else {
        inter / uni
    }
}

fn center_err(a: &AnnoBox, b: &AnnoBox) -> f64 {
    let dx = a.x + a.w / 2.0 - (b.x + b.w / 2.0);
    let dy = a.y + a.h / 2.0 - (b.y + b.h / 2.0);
    (dx * dx + dy * dy).sqrt()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(
        args.get(1)
            .expect("usage: xval_propagate <dataset_dir> [pairs]"),
    );
    let max_pairs: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(20);

    let text = std::fs::read_to_string(dir.join("annotations.json")).expect("read annotations");
    let autosave: Autosave = serde_json::from_str(&text).expect("parse annotations");
    // 文件名 → 绝对路径 key
    let mut gt: HashMap<String, Vec<AnnoBox>> = HashMap::new();
    for (k, v) in autosave.annos {
        let abs = dir.join(&k).to_string_lossy().into_owned();
        gt.insert(abs, v);
    }
    let paths: Vec<String> = gt.keys().cloned().collect();
    println!("loaded {} annotated images", paths.len());

    // 模板必须含 chat_window 锚点；按步长均匀采样 (src, dst) 对
    let templates: Vec<&String> = paths
        .iter()
        .filter(|p| gt[*p].iter().any(|b| b.tag == "chat_window"))
        .collect();
    println!("{} templates with chat_window", templates.len());
    let max_pairs = max_pairs.min(templates.len());
    let step = (templates.len() / max_pairs.max(1)).max(1);
    let mut per_tag = std::collections::BTreeMap::<String, Vec<(f64, f64)>>::new(); // tag → [(iou, center_err)]
    let mut skipped = 0usize; // 被置信度过滤掉的框
    let mut n_pairs = 0usize;

    for (i, src) in templates.iter().enumerate().step_by(step) {
        if n_pairs >= max_pairs {
            break;
        }
        let dst = templates[(i + 7) % templates.len()];
        if dst == *src {
            continue;
        }
        let boxes = &gt[*src];
        let req = PropagateRequest {
            src_path: (*src).clone(),
            boxes: boxes.clone(),
            targets: vec![dst.clone()],
            min_confidence: 0.0, // 验证用：不跳过，看原始置信度
            allow_rescale: true,
        };
        let results = propagate(&req).expect("propagate");
        let truth = &gt[dst];
        for pb in &results[0].boxes {
            // 匹配真值中同 tag 的框（一个 tag 可能多个，取 IoU 最大的）
            let best = truth
                .iter()
                .filter(|b| b.tag == pb.box_.tag)
                .map(|b| (iou(&pb.box_, b), center_err(&pb.box_, b)))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            match best {
                Some((iou_v, ce)) => {
                    per_tag
                        .entry(pb.box_.tag.clone())
                        .or_default()
                        .push((iou_v, ce));
                }
                None => {
                    per_tag
                        .entry(pb.box_.tag.clone() + " (no-truth)")
                        .or_default()
                        .push((0.0, f64::MAX));
                }
            }
        }
        skipped += boxes.len() - results[0].boxes.len();
        n_pairs += 1;
    }

    // 最差案例分析：IoU < 0.6 的框
    for (tag, vals) in &per_tag {
        if tag.ends_with("(no-truth)") {
            continue;
        }
        for (v, ce) in vals.iter().filter(|v| v.0 < 0.6) {
            println!("  BAD {} iou={:.2} cerr={:.0}", tag, v, ce);
        }
    }

    println!(
        "\n=== {} pairs, {} boxes skipped by confidence ===",
        n_pairs, skipped
    );
    println!(
        "{:<28} {:>6} {:>8} {:>8} {:>10} {:>10} {:>8}",
        "tag", "n", "iou_avg", "iou_med", "cerr_avg", "cerr_med", "iou>0.8"
    );
    let mut all: Vec<(f64, f64)> = Vec::new();
    for (tag, vals) in &per_tag {
        if tag.ends_with("(no-truth)") {
            println!("{:<28} {:>6} (tag not in truth)", tag, vals.len());
            continue;
        }
        let mut sorted: Vec<f64> = vals.iter().map(|v| v.0).collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let med = sorted[sorted.len() / 2];
        let mut cerrs: Vec<f64> = vals.iter().map(|v| v.1).collect();
        cerrs.sort_by(|a, b| a.total_cmp(b));
        let cmed = cerrs[cerrs.len() / 2];
        let good = sorted.iter().filter(|v| **v > 0.8).count();
        println!(
            "{:<28} {:>6} {:>8.3} {:>8.3} {:>10.1} {:>10.1} {:>5.0}%",
            tag,
            vals.len(),
            vals.iter().map(|v| v.0).sum::<f64>() / vals.len() as f64,
            med,
            vals.iter().map(|v| v.1).sum::<f64>() / vals.len() as f64,
            cmed,
            good as f64 / vals.len() as f64 * 100.0
        );
        all.extend(vals.iter().cloned());
    }
    if !all.is_empty() {
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!(
            "\nOVERALL: n={} iou_med={:.3} iou>0.8={:.0}% iou>0.5={:.0}%",
            all.len(),
            all[all.len() / 2].0,
            all.iter().filter(|v| v.0 > 0.8).count() as f64 / all.len() as f64 * 100.0,
            all.iter().filter(|v| v.0 > 0.5).count() as f64 / all.len() as f64 * 100.0
        );
    }
    let _ = Path::new(".").exists();
}
