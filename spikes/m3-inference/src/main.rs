//! M3 推理基座 spike（设计文档 §10 M3，不随发布物分发）。
//!
//! 这里只回答三个问题，答完就把结论写进 spike 报告，代码留着给 M4 当参照：
//!
//! 1. `ort` + ONNX Runtime 在本机能否正常初始化？DirectML EP 是否可用？失败能否回退 CPU？
//! 2. 会话初始化（含 EP 图编译）与单次推理的真实耗时量级是多少？
//! 3. 同一模型在 CPU / DirectML 上的输出是否一致（数值差异是否可忽略）？
//!
//! ```text
//! m3-inference info                                  # ORT 版本/EP 可用性
//! m3-inference sig  --model m.onnx                   # 输入输出签名（写 Stage 时要用）
//! m3-inference bench --model m.onnx [--runs 50] [--ep both]
//!                     [--shape 1,1,28,28] [--dtype f32] [--threads 2]
//! ```

use std::process::ExitCode;
use std::time::{Duration, Instant};

use ort::ep::{DirectML, ExecutionProvider, CPU};
use ort::session::Session;
use ort::value::Tensor;

/// EP 可用性（`is_available` 是 `&self` 方法且返回 Result）。
fn ep_available(label: &str) -> bool {
    let ok = match label {
        "directml" => DirectML::default().is_available(),
        _ => CPU::default().is_available(),
    };
    ok.unwrap_or(false)
}

/// 一次基准测试的结果。
struct Bench {
    label: &'static str,
    /// 同进程内连续创建多个会话时的**每个**初始化耗时：
    /// 第一个含 EP 设备/驱动初始化，后续反映纯会话建图成本。
    init_series: Vec<Duration>,
    init: Duration,
    warmup: Duration,
    p50: Duration,
    p99: Duration,
    max: Duration,
    checksum: f64,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("info") => {
            print_info();
            ExitCode::SUCCESS
        }
        Some("sig") => match opt(&args, "--model") {
            Some(path) => match signature(&path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            },
            None => usage(),
        },
        Some("bench") => {
            let Some(model) = opt(&args, "--model") else {
                return usage();
            };
            let runs: usize = opt(&args, "--runs")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let threads: usize = opt(&args, "--threads")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2);
            let seq: usize = opt(&args, "--seq").and_then(|v| v.parse().ok()).unwrap_or(32);
            let shape = parse_shape(&opt(&args, "--shape").unwrap_or_else(|| "1,1,28,28".into()));
            let ep = opt(&args, "--ep").unwrap_or_else(|| "both".to_string());
            let sessions: usize = opt(&args, "--sessions")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .max(1);

            println!("模型：{model}");
            println!(
                "图像形状覆盖：{shape:?}    令牌长度：{seq}    轮次：{runs}    intra-op 线程：{threads}\n"
            );

            let mut results = Vec::new();
            if ep == "cpu" || ep == "both" {
                match bench(&model, false, runs, threads, &shape, seq, sessions) {
                    Ok(b) => results.push(b),
                    Err(e) => println!("CPU 基准失败：{e}\n"),
                }
            }
            if ep == "dml" || ep == "both" {
                if ep_available("directml") {
                    match bench(&model, true, runs, threads, &shape, seq, sessions) {
                        Ok(b) => results.push(b),
                        Err(e) => println!("DirectML 基准失败（这正是要记录的结论）：{e}\n"),
                    }
                } else {
                    println!("DirectML 不可用，跳过\n");
                }
            }

            if results.is_empty() {
                println!("没有任何 EP 跑通");
                return ExitCode::from(1);
            }
            println!(
                "{:<12} {:>10} {:>10} {:>10} {:>10} {:>10} {:>14}",
                "EP", "init_ms", "warmup_ms", "p50_ms", "p99_ms", "max_ms", "checksum"
            );
            for b in &results {
                println!(
                    "{:<12} {:>10.1} {:>10.1} {:>10.3} {:>10.3} {:>10.3} {:>14.6}",
                    b.label,
                    ms(b.init),
                    ms(b.warmup),
                    ms(b.p50),
                    ms(b.p99),
                    ms(b.max),
                    b.checksum
                );
            }
            if sessions > 1 {
                println!(
                    "
同进程连续创建会话的初始化耗时（区分「每进程一次」与「每会话一次」）："
                );
                for b in &results {
                    let series: Vec<String> = b
                        .init_series
                        .iter()
                        .map(|d| format!("{:.1}", ms(*d)))
                        .collect();
                    println!("  {:<12} {}", b.label, series.join(" ms → ") + " ms");
                }
            }
            if results.len() == 2 {
                let delta = (results[1].checksum - results[0].checksum).abs();
                println!(
                    "\nCPU 与 DirectML 输出差异：{delta:.6}（相对量级 {:.2e}，仅作一致性参考）",
                    delta / results[0].checksum.abs().max(1e-9)
                );
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn print_info() {
    println!("=== ONNX Runtime ===");
    // ort::info() 返回版本 / git commit / 编译参数
    println!("{}", ort::info());
    println!("ort MINOR_VERSION = {}", ort::MINOR_VERSION);
    println!("\n=== 执行提供器 ===");
    println!("DirectML::is_available() = {}", ep_available("directml"));
    println!("CPU::is_available()      = {}", ep_available("cpu"));
    println!("\nGPU 设备（DirectML 用）：{:?}", ort::ep::get_gpu_device());
}

fn signature(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    let session = Session::builder()?
        .with_execution_providers([CPU::default().build()])?
        .commit_from_file(model)?;
    println!("输入：");
    for outlet in session.inputs().iter() {
        println!("  {:?}", outlet);
    }
    println!("输出：");
    for outlet in session.outputs().iter() {
        println!("  {:?}", outlet);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn bench(
    model: &str,
    use_dml: bool,
    runs: usize,
    threads: usize,
    shape: &[usize],
    seq: usize,
    sessions: usize,
) -> Result<Bench, Box<dyn std::error::Error>> {
    let label = if use_dml { "DirectML" } else { "CPU" };

    let mut init_series = Vec::with_capacity(sessions);
    let mut session = create_session(model, use_dml, threads, &mut init_series)?;
    while init_series.len() < sessions {
        // 只建会话、不做推理：测的是「挂起后唤醒重载」的真实成本（§2.4 / §5.8）
        drop(create_session(model, use_dml, threads, &mut init_series)?);
    }
    let init = *init_series.first().unwrap_or(&Duration::ZERO);

    // 从模型签名推导输入（多输入模型也能跑）
    let specs = input_specs(&session, seq, shape);
    let desc: Vec<String> = specs
        .iter()
        .map(|(n, s)| match s {
            InputSpec::Tokens(t) => format!("{n}=[1,{t}]i64"),
            InputSpec::Image(d) => format!("{n}={d:?}f32"),
        })
        .collect();
    println!("  {label} 输入：{}", desc.join(", "));

    // 第一次 run 含 EP 图编译/内存分配，单独计量（§2.3 模型重载 < 500ms 的目标看的是这一项）
    let warmup_start = Instant::now();
    let checksum = run_once(&mut session, &specs)?;
    let warmup = warmup_start.elapsed();

    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t0 = Instant::now();
        let _ = run_once(&mut session, &specs)?;
        samples.push(t0.elapsed());
    }
    samples.sort_unstable();
    let p50 = samples[runs / 2];
    let p99 = samples[(runs * 99 / 100).min(runs - 1)];
    let max = samples[runs - 1];

    Ok(Bench {
        label,
        init_series,
        init,
        warmup,
        p50,
        p99,
        max,
        checksum,
    })
}

fn create_session(
    model: &str,
    use_dml: bool,
    threads: usize,
    init_series: &mut Vec<Duration>,
) -> Result<Session, Box<dyn std::error::Error>> {
    let init_start = Instant::now();
    let builder = Session::builder()?;
    let builder = if use_dml {
        // DirectML 优先，CPU 兜底（§2.4：GPU 初始化失败自动回退）
        builder.with_execution_providers([DirectML::default().build(), CPU::default().build()])?
    } else {
        builder.with_execution_providers([CPU::default().build()])?
    };
    // §2.3 调度纪律：intra-op 线程数受限，避免打满 CPU 影响前台程序
    let session = builder
        .with_intra_threads(threads)?
        .commit_from_file(model)?;
    init_series.push(init_start.elapsed());
    Ok(session)
}

/// 单个输入的构造规格：从模型签名推导，不再依赖 CLI 猜 dtype。
#[derive(Debug, Clone, PartialEq)]
enum InputSpec {
    /// i64 令牌序列（形如 [1, seq]）：BGE 的 input_ids / attention_mask / token_type_ids
    Tokens(usize),
    /// f32 图像张量：YOLO / OCR det / OCR rec 的单输入
    Image(Vec<usize>),
}

fn input_specs(session: &Session, seq: usize, image_shape: &[usize]) -> Vec<(String, InputSpec)> {
    use ort::value::{TensorElementType, ValueType};
    session
        .inputs()
        .iter()
        .map(|outlet| {
            let dtype = outlet.dtype();
            let spec = match dtype {
                ValueType::Tensor { ty, shape, .. } if ty == &TensorElementType::Int64 => {
                    InputSpec::Tokens(seq)
                }
                ValueType::Tensor { shape, .. } => {
                    // 符号维度（-1 等）是动态维度，不能当静态形状用
                    // （i64 as usize 会把 -1 变成巨大正数，必须先判符号再取值）
                    let raw: Vec<i64> = shape.iter().copied().collect();
                    let all_static = !raw.is_empty() && raw.iter().all(|d| *d > 0);
                    if all_static {
                        InputSpec::Image(raw.iter().map(|d| *d as usize).collect())
                    } else {
                        InputSpec::Image(image_shape.to_vec())
                    }
                }
                _ => InputSpec::Image(image_shape.to_vec()),
            };
            (outlet.name().to_string(), spec)
        })
        .collect()
}

fn run_once(
    session: &mut Session,
    specs: &[(String, InputSpec)],
) -> Result<f64, Box<dyn std::error::Error>> {
    let mut named: Vec<(std::borrow::Cow<'_, str>, ort::session::SessionInputValue<'_>)> =
        Vec::with_capacity(specs.len());
    for (name, spec) in specs {
        match spec {
            InputSpec::Tokens(seq) => {
                // 按**语义**构造，而不是一律塞随机数：
                //  · attention_mask → 全 1（全部有效）
                //  · token_type_ids → 全 0（只有 2 个词表项，喂随机 id 会越界）
                //  · input_ids → 固定可复现的伪随机 id（LCG），保证两套 EP / fp32 vs int8 输入一致
                let mut state = 0x2545_f491u32;
                let ids: Vec<i64> = if name.contains("mask") {
                    vec![1i64; *seq]
                } else if name.contains("token_type") || name.contains("type_ids") {
                    vec![0i64; *seq]
                } else {
                    (0..*seq)
                        .map(|_| {
                            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (state % 20_000) as i64 + 1
                        })
                        .collect()
                };
                let value: ort::session::SessionInputValue =
                    Tensor::from_array((vec![1usize, *seq], ids.into_boxed_slice()))?.into();
                named.push((name.clone().into(), value));
            }
            InputSpec::Image(shape) => {
                let len: usize = shape.iter().product();
                let value: ort::session::SessionInputValue =
                    Tensor::from_array((shape.clone(), vec![0.0f32; len].into_boxed_slice()))?
                        .into();
                named.push((name.clone().into(), value));
            }
        }
    }
    let outputs = session.run(named)?;

    // 用输出总和当指纹：只关心「两套 EP / 两种精度」的数值是否一致，不解释语义
    let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
    Ok(data.iter().map(|v| *v as f64).sum())
}

fn parse_shape(s: &str) -> Vec<usize> {
    s.split(',')
        .filter_map(|p| p.trim().parse::<usize>().ok())
        .collect()
}

fn opt(args: &[String], key: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == key)?;
    args.get(idx + 1).cloned()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn fail(e: Box<dyn std::error::Error>) -> ExitCode {
    eprintln!("失败：{e}");
    ExitCode::from(1)
}

fn usage() -> ExitCode {
    eprintln!(
        "用法：\n  m3-inference info\n  m3-inference sig --model <onnx>\n  \
         m3-inference bench --model <onnx> [--runs 50] [--ep cpu|dml|both] \
         [--shape 1,1,640,640] [--seq 32] [--threads 2]"
    );
    ExitCode::from(2)
}
