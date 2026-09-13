use std::path::Path;

/// 支持导入的图片扩展名（FR-TAG-03，不分大小写）。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp"];

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct ImageEntry {
    pub path: String,
    pub file_name: String,
}

/// 纯函数：把用户选择的文件/目录列表展开为图片清单。
/// - 文件：扩展名匹配则收，不匹配则忽略
/// - 目录：展开一层，仅收图片文件（不递归），按文件名排序保证稳定顺序
pub fn collect_images(paths: &[String]) -> Vec<ImageEntry> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            let mut files: Vec<_> = path
                .read_dir()
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|fp| fp.is_file())
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            for fp in files {
                if let Some(entry) = image_entry(&fp) {
                    out.push(entry);
                }
            }
        } else if path.is_file() {
            if let Some(entry) = image_entry(path) {
                out.push(entry);
            }
        }
    }
    out
}

fn image_entry(path: &Path) -> Option<ImageEntry> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    Some(ImageEntry {
        path: path.to_string_lossy().into_owned(),
        file_name: path.file_name()?.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn ut_tag_02_filters_non_images() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // 大小写混合扩展名 + 各类非图片
        touch(d, "a.PNG");
        touch(d, "b.Jpeg");
        touch(d, "c.webp");
        touch(d, "note.txt");
        touch(d, "data.json");
        touch(d, "no_ext");

        let got = collect_images(&[d.to_string_lossy().into_owned()]);
        let names: Vec<&str> = got.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, ["a.PNG", "b.Jpeg", "c.webp"]);
    }

    #[test]
    fn mixed_files_and_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let img = touch(tmp.path(), "x.png");
        let txt = touch(tmp.path(), "y.txt");
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        touch(&sub, "z.png");

        let got = collect_images(&[img, txt, sub.to_string_lossy().into_owned()]);
        let names: Vec<&str> = got.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, ["x.png", "z.png"]);
    }

    /// 真实训练集冒烟：仓库内 73 张微信截图全量收进（目录不存在则跳过，
    /// 供 CI/其他检出环境下不误报）。
    #[test]
    fn real_training_set_smoke() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/training_set/wechat_snapcaptures"
        );
        let Ok(dir) = std::fs::canonicalize(dir) else {
            return;
        };
        let got = collect_images(&[dir.to_string_lossy().into_owned()]);
        assert_eq!(got.len(), 73, "训练集应为 73 张，实际 {}", got.len());
        assert!(got.iter().all(|e| e.path.ends_with(".png")));
    }
}
