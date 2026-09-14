use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 工作目录下标注文件的名字（与图片同目录）。
pub const STATE_FILE: &str = "annotations.json";

/// 自动保存的标注现场：按图片路径索引的标注框集合 + 元信息。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Autosave {
    /// 图片路径 → 标注框（磁盘上存工作目录相对路径，内存中是绝对路径）
    pub annos: HashMap<String, Vec<AnnoBox>>,
}

/// 标注框（像素坐标，原图像素系——导出时才做归一化）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnoBox {
    pub tag: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("状态文件读写失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("状态文件解析失败：{0}")]
    Parse(String),
}

/// 旧版自动保存位置：可执行文件同级的 `annotations.json`。仅用于迁移提示，
/// 现在保存路径由工作目录决定。
pub fn state_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(STATE_FILE)
}

pub fn save(autosave: &Autosave, path: &Path) -> Result<(), StateError> {
    let text = serde_json::to_string_pretty(autosave)
        .map_err(|e| StateError::Parse(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Option<Autosave>, StateError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let a: Autosave = serde_json::from_str(&text)
                .map_err(|e| StateError::Parse(e.to_string()))?;
            Ok(Some(a))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 打开（或切换）工作目录：展开图片清单；annotations.json 不存在则创建空文件，
/// 存在则加载并把 key 统一成绝对路径（磁盘上可能存的是相对路径或旧版绝对路径）。
/// 返回 (图片清单, 标注表)。
pub fn open_workspace(dir: &Path) -> Result<(Vec<crate::import::ImageEntry>, Autosave), StateError> {
    if !dir.is_dir() {
        return Err(StateError::Parse(format!("不是有效目录：{}", dir.display())));
    }
    let images = crate::import::collect_images(&[dir.to_string_lossy().into_owned()]);
    let state_file = dir.join(STATE_FILE);
    let mut autosave = match load(&state_file)? {
        Some(a) => a,
        None => {
            let empty = Autosave::default();
            save(&empty, &state_file)?;
            empty
        }
    };
    // key 相对路径（或旧绝对路径）→ 以工作目录为基的绝对路径
    let mut abs_annos: HashMap<String, Vec<AnnoBox>> = HashMap::new();
    for (key, boxes) in autosave.annos {
        let abs = to_abs_path(dir, &key);
        abs_annos.insert(abs, boxes);
    }
    autosave.annos = abs_annos;
    Ok((images, autosave))
}

/// 把磁盘上的 key 解析为绝对路径：绝对 key 原样保留，相对 key 拼到工作目录下。
fn to_abs_path(base: &Path, key: &str) -> String {
    let p = Path::new(key);
    if p.is_absolute() {
        return key.to_string();
    }
    let joined = base.join(p);
    // 不做 canonicalize：Windows 上会引入 `\\?\` 前缀，与 read_dir 产出的图片路径不一致
    joined.to_string_lossy().into_owned()
}

/// 保存到工作目录：内存中的绝对路径 key 转为相对工作目录的路径再落盘，
/// 使 json 可随目录整体移动（图片相对路径引用）。
pub fn save_workspace(
    autosave: &Autosave,
    dir: &Path,
) -> Result<(), StateError> {
    let mut rel_annos: HashMap<String, Vec<AnnoBox>> = HashMap::new();
    for (key, boxes) in &autosave.annos {
        let rel = to_rel_path(dir, key);
        rel_annos.insert(rel, boxes.clone());
    }
    save(
        &Autosave { annos: rel_annos },
        &dir.join(STATE_FILE),
    )
}

/// 绝对路径 key → 工作目录相对路径（用 `/` 分隔，跨平台一致）。
/// 不在工作目录内的路径原样保留（不应发生，但保底不丢数据）。
fn to_rel_path(base: &Path, key: &str) -> String {
    let p = Path::new(key);
    if !p.is_absolute() {
        return key.to_string();
    }
    match p.strip_prefix(base) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_of(tag: &str) -> AnnoBox {
        AnnoBox { tag: tag.into(), x: 1.0, y: 2.0, w: 3.0, h: 4.0 }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(STATE_FILE);
        let mut a = Autosave::default();
        a.annos.insert(
            "C:\\img\\a.png".into(),
            vec![AnnoBox { tag: "msg_input".into(), x: 10.0, y: 20.0, w: 100.0, h: 50.0 }],
        );
        save(&a, &path).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded, a);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(&tmp.path().join("nope.json")).unwrap().is_none());
    }

    #[test]
    fn open_workspace_creates_empty_state_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.png"), b"x").unwrap();
        let (images, autosave) = open_workspace(tmp.path()).unwrap();
        assert_eq!(images.len(), 1);
        assert!(autosave.annos.is_empty());
        assert!(tmp.path().join(STATE_FILE).exists());
    }

    #[test]
    fn workspace_roundtrip_rel_and_abs_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let _abs = tmp.path().join("a.png");
        std::fs::write(tmp.path().join("a.png"), b"x").unwrap();
        std::fs::write(tmp.path().join("b.png"), b"x").unwrap();
        std::fs::write(tmp.path().join("c.png"), b"x").unwrap();

        // 磁盘上混合：相对 key / 绝对 key（旧格式）/ 已不存在的图片 key
        let mut disk = Autosave::default();
        disk.annos.insert("a.png".into(), vec![box_of("chat_list")]);
        let abs_b = tmp.path().join("b.png");
        disk.annos
            .insert(abs_b.to_string_lossy().into_owned(), vec![box_of("msg_input")]);
        disk.annos.insert("gone.png".into(), vec![box_of("chat_target")]);
        save(&disk, &tmp.path().join(STATE_FILE)).unwrap();

        let (_, loaded) = open_workspace(tmp.path()).unwrap();
        let expect_abs = |name: &str| tmp.path().join(name).to_string_lossy().into_owned();
        assert_eq!(loaded.annos[&expect_abs("a.png")], vec![box_of("chat_list")]);
        assert_eq!(loaded.annos[&expect_abs("b.png")], vec![box_of("msg_input")]);
        assert_eq!(
            loaded.annos[&tmp.path().join("gone.png").to_string_lossy().into_owned()],
            vec![box_of("chat_target")]
        );

        // 保存回磁盘：key 全部变相对路径
        save_workspace(&loaded, tmp.path()).unwrap();
        let disk2: Autosave =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join(STATE_FILE)).unwrap())
                .unwrap();
        let mut keys: Vec<&String> = disk2.annos.keys().collect();
        keys.sort();
        assert_eq!(keys, ["a.png", "b.png", "gone.png"]);
        assert_eq!(disk2.annos["a.png"], vec![box_of("chat_list")]);
        assert_eq!(disk2.annos["b.png"], vec![box_of("msg_input")]);
    }
}
