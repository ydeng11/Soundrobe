//! Local cover-art commands and suppression parity (`electron/handlers/cover.ts`).
//! Provider downloads remain in the later provider slice.

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

const COVER_REMOVED_MARKER: &str = ".auto-tagger-cover-removed";
const COVER_NAMES: &[&str] = &[
    "cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart",
    "AlbumArt",
];
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "mp4", "wav", "ogg", "opus", "ape"];

#[tauri::command]
pub fn cover_data_url(album_path: String) -> Option<String> {
    cover_data_url_at(Path::new(&album_path))
}

#[tauri::command]
pub fn cover_set(app: AppHandle, album_path: String) -> Option<String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Choose Cover Artwork")
        .set_directory(&album_path)
        .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
        .blocking_pick_file();
    let source = picked.as_ref().and_then(FilePath::as_path)?;
    set_cover_from_path(Path::new(&album_path), source)
}

#[tauri::command]
pub fn cover_remove(album_path: String) -> bool {
    remove_cover_at(Path::new(&album_path))
}

pub fn cover_data_url_at(album_path: &Path) -> Option<String> {
    if is_cover_suppressed(album_path) {
        return None;
    }
    if let Some(path) = find_external_cover(album_path) {
        return image_data_url(&fs::read(path).ok()?, 500, 85);
    }
    let entries = fs::read_dir(album_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || !entry.file_type().ok()?.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let Ok(probe) = Probe::open(&path) else {
            continue;
        };
        let Ok(tagged) = probe
            .options(ParseOptions::new().read_properties(false))
            .read()
        else {
            continue;
        };
        for tag in tagged.tags() {
            if let Some(picture) = tag.pictures().first() {
                if let Some(url) = image_data_url(picture.data(), 500, 85) {
                    return Some(url);
                }
            }
        }
    }
    None
}

pub fn set_cover_from_path(album_path: &Path, source: &Path) -> Option<String> {
    let bytes = fs::read(source).ok()?;
    let jpeg = normalize_jpeg(&bytes, 500, 90)?;
    fs::write(album_path.join("cover.jpg"), &jpeg).ok()?;
    clear_cover_suppression(album_path).ok()?;
    image_data_url(&jpeg, 500, 85)
}

pub fn remove_cover_at(album_path: &Path) -> bool {
    let result = (|| -> std::io::Result<()> {
        if let Some(path) = find_external_cover(album_path) {
            fs::remove_file(path)?;
        }
        fs::write(album_path.join(COVER_REMOVED_MARKER), [])?;
        Ok(())
    })();
    result.is_ok()
}

fn find_external_cover(album_path: &Path) -> Option<PathBuf> {
    if !album_path.exists() {
        return None;
    }
    for name in COVER_NAMES {
        for extension in COVER_EXTENSIONS {
            let candidate = album_path.join(format!("{name}.{extension}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_cover_suppressed(album_path: &Path) -> bool {
    album_path.join(COVER_REMOVED_MARKER).exists()
}

fn clear_cover_suppression(album_path: &Path) -> std::io::Result<()> {
    let marker = album_path.join(COVER_REMOVED_MARKER);
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    Ok(())
}

fn normalize_jpeg(bytes: &[u8], max_dimension: u32, quality: u8) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let resized = if image.width() > max_dimension || image.height() > max_dimension {
        image.thumbnail(max_dimension, max_dimension)
    } else {
        image
    };
    encode_jpeg(&resized, quality)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality)
        .encode_image(image)
        .ok()?;
    Some(output)
}

fn image_data_url(bytes: &[u8], max_dimension: u32, quality: u8) -> Option<String> {
    let jpeg = normalize_jpeg(bytes, max_dimension, quality)?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImage, ImageFormat, Rgba};
    use lofty::id3::v2::Id3v2Tag;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::TagExt;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "auto-tagger-cover-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut image = DynamicImage::new_rgba8(width, height);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Png).unwrap();
        output.into_inner()
    }

    #[test]
    fn set_cover_resizes_writes_jpeg_and_clears_suppression() {
        let root = root();
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let source = root.join("source.png");
        fs::write(&source, png(1000, 250)).unwrap();

        let url = set_cover_from_path(&album, &source).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        assert!(!album.join(COVER_REMOVED_MARKER).exists());
        let cover = fs::read(album.join("cover.jpg")).unwrap();
        assert_eq!(&cover[..2], &[0xff, 0xd8]);
        let decoded = image::load_from_memory(&cover).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (500, 125));
        assert!(cover_data_url_at(&album).is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_deletes_first_external_cover_and_suppresses_remaining_sources() {
        let root = root();
        fs::write(root.join("cover.jpg"), png(2, 2)).unwrap();
        fs::write(root.join("front.png"), png(2, 2)).unwrap();
        assert!(remove_cover_at(&root));
        assert!(!root.join("cover.jpg").exists());
        assert!(root.join("front.png").exists());
        assert!(root.join(COVER_REMOVED_MARKER).exists());
        assert_eq!(cover_data_url_at(&root), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn data_url_falls_back_to_embedded_cover() {
        let root = root();
        let track = root.join("track.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track,
        )
        .unwrap();
        let mut tag = Id3v2Tag::new();
        tag.insert_picture(
            Picture::unchecked(png(4, 4))
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&track, lofty::config::WriteOptions::new())
            .unwrap();
        let url = cover_data_url_at(&root).expect("valid embedded artwork");
        assert!(url.starts_with("data:image/jpeg;base64,"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_external_image_and_missing_album_fail_closed() {
        let root = root();
        fs::write(root.join("cover.jpg"), b"not an image").unwrap();
        assert_eq!(cover_data_url_at(&root), None);
        assert_eq!(set_cover_from_path(&root, &root.join("missing.png")), None);
        let missing = root.join("missing-album");
        assert_eq!(cover_data_url_at(&missing), None);
        assert!(!remove_cover_at(&missing));
        fs::remove_dir_all(root).unwrap();
    }
}
