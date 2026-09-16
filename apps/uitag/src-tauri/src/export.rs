use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::cmd::ExportRequest;
use crate::coords::to_yolo_line;
use crate::state::AnnoBox;

/// 把数据集写成 zip（UT-TAG-03）：
/// - `images/<stem>.<原扩展名>`：原样拷贝字节
/// - `labels/<stem>.txt`：每框一行 YOLO `class cx cy w h`；无框图片写空文件（负样本）
/// - `classes.txt`：tags.json 顺序，每行一个 name（class id = 行号）
/// - `tags.json`：配置快照
pub fn write_dataset_zip(req: &ExportRequest, dest: &Path) -> Result<std::path::PathBuf, String> {
    let stem_of = |src: &str| -> Result<String, String> {
        Path::new(src)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| format!("非法图片路径：{src}"))
    };
    let ext_of = |src: &str| -> Result<String, String> {
        Path::new(src)
            .extension()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .ok_or_else(|| format!("图片缺少扩展名：{src}"))
    };

    // class id 由 tags 顺序决定（enabled 的才参与；保持与 classes.txt 一致）
    let class_ids: HashMap<&str, usize> = req
        .tags
        .tags
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i))
        .collect();

    // stem 冲突检查：不同图片同 stem 会互相覆盖标签文件
    let mut seen = std::collections::HashSet::new();
    for img in &req.images {
        let stem = stem_of(&img.src)?;
        if !seen.insert(stem.clone()) {
            return Err(format!("文件名主干重复：{stem}"));
        }
    }

    let file = std::fs::File::create(dest).map_err(|e| format!("创建导出文件失败：{e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default();

    for img in &req.images {
        let stem = stem_of(&img.src)?;
        let bytes =
            std::fs::read(&img.src).map_err(|e| format!("读取图片失败 {}: {e}", img.src))?;

        // 前端只对浏览过的图片有尺寸缓存；缺失时从图片文件头实测（避免 0 尺寸导致导出失败）
        let (iw, ih) = if img.width > 0 && img.height > 0 {
            (img.width, img.height)
        } else {
            image::image_dimensions(&img.src)
                .map_err(|e| format!("读取图片尺寸失败 {}: {e}", img.src))?
        };

        let img_name = format!("images/{stem}.{}", ext_of(&img.src)?);
        zip.start_file(&img_name, opts).map_err(zip_err)?;
        zip.write_all(&bytes)
            .map_err(|e| format!("zip 写入失败：{e}"))?;

        let label_name = format!("labels/{stem}.txt");
        zip.start_file(&label_name, opts).map_err(zip_err)?;
        let empty: Vec<AnnoBox> = Vec::new();
        // 无标注的图片也写空 labels 文件 = YOLO 负样本
        for b in req.annos.get(&stem).unwrap_or(&empty) {
            let Some(class) = class_ids.get(b.tag.as_str()) else {
                return Err(format!("标注 tag「{}」不在 tags 表中", b.tag));
            };
            let Some(line) = to_yolo_line(b, iw, ih, *class) else {
                return Err(format!("图片尺寸非法：{stem}"));
            };
            zip.write_all(line.as_bytes())
                .map_err(|e| format!("zip 写入失败：{e}"))?;
            zip.write_all(b"\n")
                .map_err(|e| format!("zip 写入失败：{e}"))?;
        }
    }

    zip.start_file("classes.txt", opts).map_err(zip_err)?;
    for t in &req.tags.tags {
        zip.write_all(t.name.as_bytes())
            .map_err(|e| format!("zip 写入失败：{e}"))?;
        zip.write_all(b"\n")
            .map_err(|e| format!("zip 写入失败：{e}"))?;
    }

    zip.start_file("tags.json", opts).map_err(zip_err)?;
    let tags_snapshot =
        serde_json::to_string_pretty(&req.tags).map_err(|e| format!("序列化 tags 失败：{e}"))?;
    zip.write_all(tags_snapshot.as_bytes())
        .map_err(|e| format!("zip 写入失败：{e}"))?;

    zip.finish().map_err(zip_err)?;
    Ok(dest.to_path_buf())
}

fn zip_err(e: zip::result::ZipError) -> String {
    format!("zip 写入失败：{e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::ExportImage;
    use crate::tags::TagsConfig;
    use zip::ZipArchive;

    fn fixture() -> (tempfile::TempDir, ExportRequest) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let png = dir.join("shot1.png");
        std::fs::write(&png, b"fake-png-bytes").unwrap();
        let jpg = dir.join("shot2.jpg");
        std::fs::write(&jpg, b"fake-jpg-bytes").unwrap();
        let unlabeled = dir.join("shot3.png");
        std::fs::write(&unlabeled, b"more-bytes").unwrap();

        let tags = TagsConfig::builtin();
        let mut annos = std::collections::HashMap::new();
        annos.insert(
            "shot1".to_string(),
            vec![
                AnnoBox {
                    tag: "chat_list".into(),
                    x: 0.0,
                    y: 0.0,
                    w: 265.0,
                    h: 774.0,
                },
                AnnoBox {
                    tag: "msg_input".into(),
                    x: 265.0,
                    y: 700.0,
                    w: 265.0,
                    h: 74.0,
                },
            ],
        );
        // shot2 无标注 → 负样本空文件

        let req = ExportRequest {
            images: vec![
                ExportImage {
                    src: png.to_string_lossy().into_owned(),
                    width: 530,
                    height: 774,
                },
                ExportImage {
                    src: jpg.to_string_lossy().into_owned(),
                    width: 1068,
                    height: 766,
                },
                ExportImage {
                    src: unlabeled.to_string_lossy().into_owned(),
                    width: 530,
                    height: 774,
                },
            ],
            annos,
            tags,
            dest: String::new(),
        };
        (tmp, req)
    }

    /// UT-TAG-03：zip 结构 / classes 顺序 / YOLO 行值 / tags.json 快照。
    #[test]
    fn ut_tag_03_zip_structure_and_content() {
        let (tmp, mut req) = fixture();
        let out = tmp.path().join("dataset.zip");
        req.dest = out.to_string_lossy().into_owned();

        let got = write_dataset_zip(&req, &out).unwrap();
        assert_eq!(got, out);

        let f = std::fs::File::open(&out).unwrap();
        let mut zip = ZipArchive::new(f).unwrap();
        let mut names: Vec<String> = zip.file_names().map(String::from).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "classes.txt",
                "images/shot1.png",
                "images/shot2.jpg",
                "images/shot3.png",
                "labels/shot1.txt",
                "labels/shot2.txt",
                "labels/shot3.txt",
                "tags.json",
            ]
        );

        // classes.txt 顺序 = tags 顺序
        let classes = read_entry(&mut zip, "classes.txt");
        assert_eq!(classes, "chat_list\nchat_window\nchat_target\nmsg_input\n");

        // YOLO 行值：class id + 归一化（530×774 图，msg_input 框 265,700,265,74 →
        // cx=0.75, cy=737/774≈0.9521964, w=0.5, h=74/774≈0.0956072）
        let labels = read_entry(&mut zip, "labels/shot1.txt");
        let lines: Vec<&str> = labels.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("0 0.25"), "{}", lines[0]);
        assert!(
            lines[1].starts_with("3 0.7500000000 0.9521963824 0.5000000000 0.0956072351"),
            "{}",
            lines[1]
        );

        // 无标注 → 空文件
        assert_eq!(read_entry(&mut zip, "labels/shot2.txt"), "");
        assert_eq!(read_entry(&mut zip, "labels/shot3.txt"), "");

        // 图片字节原样拷贝
        assert_eq!(read_entry(&mut zip, "images/shot1.png"), "fake-png-bytes");
        assert_eq!(read_entry(&mut zip, "images/shot2.jpg"), "fake-jpg-bytes");

        // tags.json 快照可解析回 TagsConfig
        let snap = read_entry(&mut zip, "tags.json");
        assert_eq!(TagsConfig::parse(&snap).unwrap(), TagsConfig::builtin());
    }

    #[test]
    fn rejects_duplicate_stems() {
        let (tmp, mut req) = fixture();
        // 子目录里的同名文件：stem 与 shot1 撞车
        let sub = tmp.path().join("nested");
        std::fs::create_dir(&sub).unwrap();
        let dup = sub.join("shot1.png");
        std::fs::write(&dup, b"x").unwrap();
        req.images.push(ExportImage {
            src: dup.to_string_lossy().into_owned(),
            width: 10,
            height: 10,
        });
        let out = tmp.path().join("d.zip");
        let err = write_dataset_zip(&req, &out).unwrap_err();
        assert!(err.contains("重复"), "{err}");
    }

    #[test]
    fn rejects_unknown_tag() {
        let (tmp, mut req) = fixture();
        req.annos.insert(
            "shot3".to_string(),
            vec![AnnoBox {
                tag: "no_such_tag".into(),
                x: 1.0,
                y: 1.0,
                w: 2.0,
                h: 2.0,
            }],
        );
        let out = tmp.path().join("d.zip");
        let err = write_dataset_zip(&req, &out).unwrap_err();
        assert!(err.contains("no_such_tag"), "{err}");
    }

    /// 前端尺寸缓存缺失（width/height = 0，未浏览过的图片）时，
    /// 从图片文件头实测尺寸，不再导出失败。
    #[test]
    fn zero_dims_fall_back_to_image_header() {
        let (tmp, mut req) = fixture();
        let path = tmp.path().join("shot3.png");
        image::RgbImage::from_pixel(4, 6, image::Rgb([10, 20, 30]))
            .save(&path)
            .unwrap();
        for img in req.images.iter_mut() {
            if img.src.ends_with("shot3.png") {
                img.width = 0;
                img.height = 0;
            }
        }
        req.annos.insert(
            "shot3".to_string(),
            vec![AnnoBox {
                tag: "msg_input".into(),
                x: 1.0,
                y: 1.0,
                w: 2.0,
                h: 2.0,
            }],
        );
        let out = tmp.path().join("d.zip");
        write_dataset_zip(&req, &out).unwrap();
    }

    fn read_entry(zip: &mut ZipArchive<std::fs::File>, name: &str) -> String {
        let mut f = zip.by_name(name).unwrap();
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut f, &mut buf).unwrap();
        buf
    }
}
