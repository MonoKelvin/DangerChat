//! JSON 配置序列化集成测试

use dc_core::{ConfigCenter, ConfigField, ConfigType, ConfigValue};

#[test]
fn config_center_json_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("config-json-test-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let config = ConfigCenter::open(&tmp).expect("打开失败");

    // 注册测试 schema
    config
        .register_module(
            "test",
            vec![ConfigField {
                key: "test.value".into(),
                ty: ConfigType::Int { min: 0, max: 100 },
                default: ConfigValue::Int(42),
                label: "测试值".into(),
                help: "测试用配置".into(),
                group: "测试".into(),
                owner: "test".into(),
            }],
        )
        .expect("注册失败");

    config.finalize().expect("finalize 失败");

    // 修改配置
    config
        .set("test.value", ConfigValue::Int(88))
        .expect("set 失败");

    // 重新加载
    config.reload().expect("reload 失败");

    // 验证值持久化成功
    let snapshot = config.snapshot();
    assert_eq!(snapshot.i64_or("test.value", 0), 88);

    // 验证文件是 JSON 格式
    let text = std::fs::read_to_string(&tmp).expect("读取失败");
    assert!(text.contains('{'));
    assert!(text.contains("test"));

    let _ = std::fs::remove_file(&tmp);
}
