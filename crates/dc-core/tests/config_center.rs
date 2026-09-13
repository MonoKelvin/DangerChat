//! UT-CORE-01 ~ UT-CORE-04：配置中心（设计文档 §5.1 测试矩阵）

use dc_core::config::{ConfigType, ConfigValue};
use dc_core::{ConfigCenter, ConfigField};
use tempfile::TempDir;

fn fields() -> Vec<ConfigField> {
    vec![
        ConfigField::new(
            "guard.enabled",
            ConfigType::Bool,
            ConfigValue::Bool(true),
            "启用守护",
            "guard",
            "guard",
        ),
        ConfigField::new(
            "guard.verdict_ttl_ms",
            ConfigType::Int {
                min: 100,
                max: 10_000,
            },
            ConfigValue::Int(2000),
            "判定有效期",
            "guard",
            "guard",
        ),
        ConfigField::new(
            "sem.threshold.formal",
            ConfigType::Float { min: 0.0, max: 1.0 },
            ConfigValue::Float(0.55),
            "正式场景阈值",
            "sem",
            "sem",
        ),
        ConfigField::new(
            "guard.send_key",
            ConfigType::Enum {
                options: vec!["enter".into(), "ctrl+enter".into()],
            },
            ConfigValue::Str("enter".into()),
            "发送快捷键",
            "guard",
            "guard",
        ),
        ConfigField::new(
            "ocr.noise_words",
            ConfigType::StrList,
            ConfigValue::StrList(vec!["发送".into()]),
            "噪声词",
            "ocr",
            "ocr",
        ),
    ]
}

/// 按真实装配方式注册：每个模块只声明自己的配置项（owner 由注册方决定）。
fn open(dir: &TempDir) -> ConfigCenter {
    let center = ConfigCenter::open(dir.path().join("config.toml")).expect("open");
    let all = fields();
    for module in ["guard", "sem", "ocr"] {
        let own: Vec<ConfigField> = all
            .iter()
            .filter(|f| f.key.starts_with(module))
            .cloned()
            .collect();
        center.register_module(module, own).expect("register");
    }
    center
}

/// UT-CORE-01 schema 注册/读取/默认值回填往返
#[test]
fn ut_core_01_schema_defaults_roundtrip() {
    let dir = TempDir::new().unwrap();
    let center = open(&dir);
    let snap = center.finalize().expect("finalize");

    // 默认值回填
    assert!(snap.bool_or("guard.enabled", false));
    assert_eq!(snap.i64_or("guard.verdict_ttl_ms", 0), 2000);
    assert!((snap.f64_or("sem.threshold.formal", 0.0) - 0.55).abs() < 1e-9);
    assert_eq!(snap.str_or("guard.send_key", ""), "enter");
    assert_eq!(
        snap.str_list_or("ocr.noise_words", &[]),
        vec!["发送".to_string()]
    );

    // 规范化落盘：schema 键全部写入文件
    let text = std::fs::read_to_string(center.path()).expect("config written");
    assert!(text.contains("verdict_ttl_ms = 2000"), "text = {text}");
    assert!(text.contains("[guard]"), "text = {text}");

    // 往返：改值 → 重新打开同一文件 → 值保留
    center
        .set("guard.verdict_ttl_ms", ConfigValue::Int(3000))
        .expect("set");
    center
        .set("guard.send_key", ConfigValue::Str("ctrl+enter".into()))
        .expect("set");

    let reopened = open(&dir);
    let snap2 = reopened.finalize().expect("finalize again");
    assert_eq!(snap2.i64_or("guard.verdict_ttl_ms", 0), 3000);
    assert_eq!(snap2.str_or("guard.send_key", ""), "ctrl+enter");
    assert_eq!(reopened.get("guard.enabled"), Some(ConfigValue::Bool(true)));

    // schema 清单可枚举（前端渲染依据）
    let schema = reopened.schema();
    assert_eq!(schema.len(), 5);
    assert!(schema
        .iter()
        .any(|f| f.key == "guard.send_key" && f.ty.kind_name() == "enum"));
}

/// UT-CORE-02 非法类型/越界值 set 被拒绝且不落盘
#[test]
fn ut_core_02_invalid_set_rejected_without_persisting() {
    let dir = TempDir::new().unwrap();
    let center = open(&dir);
    center.finalize().expect("finalize");
    let before = std::fs::read_to_string(center.path()).unwrap();

    // 越界
    assert!(center
        .set("guard.verdict_ttl_ms", ConfigValue::Int(99_999))
        .is_err());
    // 类型不符
    assert!(center
        .set("guard.enabled", ConfigValue::Str("yes".into()))
        .is_err());
    // 枚举外取值
    assert!(center
        .set("guard.send_key", ConfigValue::Str("space".into()))
        .is_err());
    // 未知键
    assert!(center.set("nope.key", ConfigValue::Bool(true)).is_err());

    assert_eq!(
        center.get("guard.verdict_ttl_ms"),
        Some(ConfigValue::Int(2000))
    );
    assert_eq!(
        center.get("guard.send_key"),
        Some(ConfigValue::Str("enter".into()))
    );
    let after = std::fs::read_to_string(center.path()).unwrap();
    assert_eq!(before, after, "非法 set 不得改动已落盘配置");

    // 合法 set 才产生变更广播
    let rx = center.subscribe();
    center
        .set("guard.verdict_ttl_ms", ConfigValue::Int(2500))
        .expect("legal set");
    let changed = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .unwrap();
    assert_eq!(changed.key, "guard.verdict_ttl_ms");
    assert_eq!(changed.value, ConfigValue::Int(2500));
    assert!(rx.try_recv().is_err(), "非法 set 不应广播");
}

/// UT-CORE-03 损坏配置文件自动恢复
#[test]
fn ut_core_03_corrupt_config_recovers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[[[ this is not toml").unwrap();

    let center = ConfigCenter::open(&path).expect("open on corrupt file");
    let recoveries = center.recoveries();
    assert_eq!(recoveries.len(), 1, "应记录一次恢复");
    assert!(recoveries[0].backup.exists(), "原文件应被备份");
    assert_eq!(
        std::fs::read_to_string(&recoveries[0].backup).unwrap(),
        "[[[ this is not toml"
    );

    center.register_module("guard", fields()).unwrap();
    let snap = center.finalize().expect("rebuild with defaults");
    assert_eq!(snap.i64_or("guard.verdict_ttl_ms", 0), 2000, "以默认值重建");
    assert!(snap.bool_or("guard.enabled", false));

    // 恢复后的文件是合法 TOML
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.parse::<toml::Value>().is_ok(), "text = {text}");
}

/// UT-CORE-04 并发写配置无丢失（rename 原子性）
#[test]
fn ut_core_04_concurrent_writes_are_atomic() {
    let dir = TempDir::new().unwrap();
    let center = std::sync::Arc::new(open(&dir));
    center.finalize().unwrap();

    let keys = [
        "guard.verdict_ttl_ms",
        "sem.threshold.formal",
        "guard.enabled",
    ];
    // 三个可写字段：并发各写 30 次，外加若干次非法写（不应破坏文件）
    let mut handles = Vec::new();
    for &key in &keys {
        let center = std::sync::Arc::clone(&center);
        let key = key.to_string();
        handles.push(std::thread::spawn(move || {
            for i in 0..30 {
                let value = match key.as_str() {
                    "guard.enabled" => ConfigValue::Bool(i % 2 == 0),
                    "sem.threshold.formal" => ConfigValue::Float(0.1 + (i as f64) / 100.0),
                    _ => ConfigValue::Int(4000 + i),
                };
                center.set(&key, value).expect("set");
            }
        }));
    }
    for &key in &keys {
        let center = std::sync::Arc::clone(&center);
        let key = key.to_string();
        handles.push(std::thread::spawn(move || {
            for _ in 0..20 {
                let _ = center.set(&key, ConfigValue::Str("bogus".into()));
            }
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }

    // 重新打开：文件必须合法且三个键都是各自线程最后一次写入的值
    let reopened = open(&dir);
    let snap = reopened.finalize().expect("finalize");
    assert_eq!(
        snap.i64_or("guard.verdict_ttl_ms", 0),
        4029,
        "每个键最后一次写入生效"
    );
    assert!((snap.f64_or("sem.threshold.formal", 0.0) - 0.39).abs() < 1e-9);
    assert!(!snap.bool_or("guard.enabled", true), "i=29 → 奇数次写入");
    let raw = std::fs::read_to_string(reopened.path()).unwrap();
    assert!(
        raw.parse::<toml::Value>().is_ok(),
        "并发写后文件仍合法: {raw}"
    );
    // 无临时文件残留
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "残留 tmp: {leftovers:?}");
}

/// 补充：注册键冲突是装配期错误（§5.1 init 期 panic）
#[test]
#[should_panic(expected = "配置键")]
fn duplicate_key_across_modules_panics() {
    let dir = TempDir::new().unwrap();
    let center = ConfigCenter::open(dir.path().join("config.toml")).unwrap();
    let field = ConfigField::new(
        "guard.enabled",
        ConfigType::Bool,
        ConfigValue::Bool(true),
        "启用",
        "guard",
        "guard",
    );
    center
        .register_module("guard", vec![field.clone()])
        .unwrap();
    center.register_module("intruder", vec![field]).unwrap();
}
