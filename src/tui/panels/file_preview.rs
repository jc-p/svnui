//! 按文件类型及大小分流预览；压缩包和大文件只返回提示，不进行内容解析。

use std::{
    fs::File,
    io::{self, Cursor, Read},
    path::Path,
};

use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// 所有本地预览共用上限，包括图片和办公文档。
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub enum FileContent {
    Text(String),
    Document(String),
    Image { pixels: DynamicImage, bytes: u64 },
    Disabled(String),
}

/// 先限制输入字节，再识别格式；文档扩展名只用于分流现有的专用解析器。
pub fn load(path: &Path) -> io::Result<FileContent> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Ok(FileContent::Disabled("此路径不是普通文件".into()));
    }
    if meta.len() > MAX_BYTES {
        return Ok(large_file(meta.len()));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_archive_extension(&ext) {
        return Ok(FileContent::Disabled(format!(
            "压缩包预览已关闭 · {} 字节",
            meta.len()
        )));
    }
    // 多读一字节确认读取期间文件没有越过上限，不把增长后的文件整份载入内存。
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Ok(large_file(bytes.len() as u64));
    }

    let format = image::guess_format(&bytes).ok();
    if format.is_some()
        || matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "jpe" | "webp" | "gif" | "bmp" | "ico"
        )
    {
        return Ok(match decode_image(&bytes, format) {
            Ok(pixels) => FileContent::Image {
                pixels,
                bytes: bytes.len() as u64,
            },
            Err(reason) => FileContent::Disabled(format!("无法预览图片：{reason}")),
        });
    }

    // xlsx/docx 本身也是 ZIP；只让明确支持的办公文档绕过归档和二进制拦截。
    let office = matches!(
        ext.as_str(),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "docx"
    );
    if !office && is_archive_signature(&bytes) {
        return Ok(FileContent::Disabled(format!(
            "压缩包预览已关闭 · {} 字节",
            bytes.len()
        )));
    }
    if !office && bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(FileContent::Disabled(format!(
            "二进制文件 · {} 字节 · 不支持内容预览",
            bytes.len()
        )));
    }
    #[cfg(feature = "docs")]
    if super::document::is_document(path) {
        return Ok(match super::document::extract(path) {
            Some(text) => FileContent::Document(text),
            None => FileContent::Disabled("无法解析该文档：格式不受支持或文件已损坏".into()),
        });
    }
    #[cfg(not(feature = "docs"))]
    if office {
        return Ok(FileContent::Disabled(
            "此构建未启用办公文档预览（docs feature）".into(),
        ));
    }

    Ok(FileContent::Text(
        String::from_utf8_lossy(&bytes).into_owned(),
    ))
}

fn large_file(bytes: u64) -> FileContent {
    FileContent::Disabled(format!("大文件预览已关闭 · {bytes} 字节 · 上限 2 MiB"))
}

fn is_archive_extension(ext: &str) -> bool {
    matches!(
        ext,
        "zip"
            | "zipx"
            | "rar"
            | "7z"
            | "tar"
            | "gz"
            | "tgz"
            | "bz2"
            | "tbz2"
            | "xz"
            | "txz"
            | "zst"
            | "tzst"
            | "lz"
            | "lz4"
            | "cab"
            | "jar"
            | "war"
            | "ear"
            | "apk"
    )
}

fn is_archive_signature(bytes: &[u8]) -> bool {
    [
        b"PK\x03\x04".as_slice(),
        b"PK\x05\x06",
        b"PK\x07\x08",
        b"\x1f\x8b",
        b"Rar!",
        b"7z\xbc\xaf\x27\x1c",
        b"BZh",
        b"\xfd7zXZ\0",
        b"\x28\xb5\x2f\xfd",
        b"\x04\x22\x4d\x18",
    ]
    .iter()
    .any(|magic| bytes.starts_with(magic))
        || bytes.get(257..262) == Some(b"ustar".as_slice())
}

fn decode_image(bytes: &[u8], format: Option<ImageFormat>) -> Result<DynamicImage, String> {
    let format = format.ok_or("图片格式无法识别或文件已损坏")?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Gif
            | ImageFormat::Bmp
            | ImageFormat::Ico
    ) {
        return Err("暂不支持此图片格式".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    // 压缩后的图片很小也可能有巨大尺寸，限制解码内存及单边像素数。
    let mut limits = Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disabled(path: &Path) -> String {
        match load(path).unwrap() {
            FileContent::Disabled(reason) => reason,
            _ => panic!("应阻止预览：{}", path.display()),
        }
    }

    #[test]
    fn large_files_are_blocked_before_text_image_or_document_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        for ext in ["txt", "png", "docx", "xlsx", "zip"] {
            let path = tmp.path().join(format!("large.{ext}"));
            File::create(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
            assert!(disabled(&path).contains("大文件预览已关闭"));
        }
        let path = tmp.path().join("boundary.txt");
        std::fs::write(&path, vec![b'a'; MAX_BYTES as usize]).unwrap();
        assert!(
            matches!(load(&path).unwrap(), FileContent::Text(text) if text.len() as u64 == MAX_BYTES)
        );
    }

    #[test]
    fn archives_are_blocked_by_extension_or_signature() {
        let tmp = tempfile::tempdir().unwrap();
        for ext in ["zip", "tar", "gz", "rar", "7z", "xz", "zst", "JAR"] {
            let path = tmp.path().join(format!("archive.{ext}"));
            std::fs::write(&path, b"not parsed").unwrap();
            assert!(disabled(&path).contains("压缩包预览已关闭"));
        }
        for prefix in [
            b"PK\x03\x04".as_slice(),
            b"\x1f\x8b",
            b"Rar!",
            b"7z\xbc\xaf\x27\x1c",
            b"BZh",
            b"\xfd7zXZ\0",
            b"\x28\xb5\x2f\xfd",
        ] {
            for ext in ["txt", "csv"] {
                let path = tmp.path().join(format!("renamed.{ext}"));
                std::fs::write(&path, prefix).unwrap();
                assert!(disabled(&path).contains("压缩包预览已关闭"));
            }
        }
    }

    #[test]
    fn common_images_decode_by_content_and_invalid_images_have_a_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let pixels =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([220, 30, 50])));
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::WebP,
            ImageFormat::Gif,
            ImageFormat::Bmp,
            ImageFormat::Ico,
        ] {
            let mut buffer = Cursor::new(Vec::new());
            let encoded_pixels = if format == ImageFormat::Ico {
                DynamicImage::ImageRgba8(pixels.to_rgba8())
            } else {
                pixels.clone()
            };
            encoded_pixels.write_to(&mut buffer, format).unwrap();
            let path = tmp.path().join("image-without-extension");
            std::fs::write(&path, buffer.into_inner()).unwrap();
            match load(&path).unwrap() {
                FileContent::Image { pixels, .. } => {
                    assert_eq!((pixels.width(), pixels.height()), (8, 8))
                }
                FileContent::Disabled(reason) => panic!("图片格式未正确分流：{format:?}，{reason}"),
                _ => panic!("图片格式未正确分流：{format:?}"),
            }
        }
        let path = tmp.path().join("broken.png");
        std::fs::write(&path, b"broken image").unwrap();
        assert!(disabled(&path).contains("无法预览图片"));
        DynamicImage::new_rgb8(9000, 1).save(&path).unwrap();
        assert!(disabled(&path).contains("无法预览图片"));
    }

    #[test]
    fn common_text_and_document_previews_are_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        for ext in ["rs", "js", "json", "md", "html", "svg", "txt"] {
            let path = tmp.path().join(format!("source.{ext}"));
            std::fs::write(&path, "文本内容\n").unwrap();
            assert!(
                matches!(load(&path).unwrap(), FileContent::Text(text) if text == "文本内容\n")
            );
        }
        let path = tmp.path().join("table.csv");
        std::fs::write(&path, "name,value\n样例,123\n").unwrap();
        #[cfg(feature = "docs")]
        assert!(
            matches!(load(&path).unwrap(), FileContent::Document(text) if text.contains("样例"))
        );
        #[cfg(not(feature = "docs"))]
        assert!(matches!(load(&path).unwrap(), FileContent::Text(text) if text.contains("样例")));
        let path = tmp.path().join("binary.bin");
        std::fs::write(&path, b"data\0binary").unwrap();
        assert!(disabled(&path).contains("二进制文件"));
    }
}
