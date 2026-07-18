//! LRCLIB lyrics fetch command.

use crate::commands::mutations::{write_track_dispatch, Patch, TrackPatch};
use crate::commands::tracks::read_track_metadata;
use crate::error::ApiError;
use crate::state::config::ConfigState;
use crate::state::write_queue::WriteQueue;
use chardetng::EncodingDetector;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tauri::State;

pub const DEFAULT_BASE_URL: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!(
    "soundrobe/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/ydeng11/Soundrobe)"
);
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "wav", "ogg", "opus", "aiff", "ape",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LyricsResponse {
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
    #[allow(dead_code)]
    instrumental: Option<bool>,
}

#[tauri::command]
pub async fn lyrics_fetch(
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: Option<f64>,
) -> Option<String> {
    fetch_lyrics_at(
        DEFAULT_BASE_URL,
        &track_name,
        &artist_name,
        album_name.as_deref(),
        duration,
    )
    .await
}

#[tauri::command]
pub async fn album_download_lyrics(
    album_path: String,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<usize, ApiError> {
    let base_url = config
        .raw()
        .lyrics_api_url
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    Ok(download_album_lyrics_at(Path::new(&album_path), &base_url, &queue).await)
}

pub async fn download_album_lyrics_at(
    album_path: &Path,
    base_url: &str,
    queue: &WriteQueue,
) -> usize {
    apply_album_lyrics_at(album_path, Some(base_url), queue).await
}

pub async fn apply_album_lyrics_at(
    album_path: &Path,
    base_url: Option<&str>,
    queue: &WriteQueue,
) -> usize {
    let Ok(entries) = fs::read_dir(album_path) else {
        return 0;
    };
    let mut audio_files = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return 0;
        };
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Ok(file_type) = entry.file_type() else {
            return 0;
        };
        if file_type.is_file() && AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            audio_files.push(path);
        }
    }
    audio_files.sort();
    let mut jobs = Vec::new();
    for path in audio_files {
        let mut lyrics = read_local_lyrics(&path);
        if lyrics.is_none() {
            if let (Some(base_url), Ok(metadata)) = (base_url, read_track_metadata(&path)) {
                if let (Some(title), Some(artist)) = (metadata.title, metadata.artist) {
                    lyrics = fetch_lyrics_at(
                        base_url,
                        &title,
                        &artist,
                        metadata.album.as_deref(),
                        (metadata.duration > 0.0).then_some(metadata.duration.round()),
                    )
                    .await;
                }
            }
        }
        if let Some(lyrics) = lyrics.filter(|lyrics| !lyrics.is_empty()) {
            jobs.push((path, lyrics));
        }
    }
    if jobs.is_empty() {
        return 0;
    }
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                jobs.into_iter()
                    .filter(|(path, lyrics)| {
                        let patch = TrackPatch {
                            lyrics: Patch::Value(lyrics.clone()),
                            ..TrackPatch::default()
                        };
                        write_track_dispatch(path, &patch).is_ok()
                    })
                    .count()
            })
            .await
            .unwrap_or(0)
        })
        .await
}

pub fn read_local_lyrics(file_path: &Path) -> Option<String> {
    for extension in ["lrc", "txt"] {
        let path = file_path.with_extension(extension);
        if let Ok(bytes) = fs::read(path) {
            return Some(normalize_lyrics_encoding(&bytes));
        }
    }
    None
}

pub fn normalize_lyrics_encoding(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&units);
    }
    if let Ok(utf8) = std::str::from_utf8(bytes) {
        return utf8.to_string();
    }
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

pub async fn fetch_lyrics_at(
    base_url: &str,
    track_name: &str,
    artist_name: &str,
    album_name: Option<&str>,
    duration: Option<f64>,
) -> Option<String> {
    if track_name.is_empty() || artist_name.is_empty() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;
    let mut query = vec![
        ("track_name", track_name.to_string()),
        ("artist_name", artist_name.to_string()),
    ];
    if let Some(album_name) = album_name.filter(|value| !value.is_empty()) {
        query.push(("album_name", album_name.to_string()));
    }
    if let Some(duration) = duration.filter(|value| *value > 0.0) {
        query.push(("duration", duration.round().to_string()));
    }
    let url = format!("{}/get", base_url.trim_end_matches('/'));
    let response = client.get(url).query(&query).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<LyricsResponse>().await.ok()?;
    body.synced_lyrics
        .or(body.plain_lyrics)
        .filter(|lyrics| !lyrics.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn server(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let body = body.to_string();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let _ = send.send(String::from_utf8_lossy(&request[..read]).into_owned());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}/api/"), receive)
    }

    fn temp_root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-lyrics-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn local_lyrics_prefers_lrc_and_decodes_utf16_boms() {
        let root = temp_root();
        let audio = root.join("song.mp3");
        fs::write(&audio, b"").unwrap();
        fs::write(audio.with_extension("txt"), "fallback").unwrap();
        let text = "[00:01]你好";
        let mut utf16 = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(audio.with_extension("lrc"), utf16).unwrap();
        assert_eq!(read_local_lyrics(&audio).as_deref(), Some(text));
        let be = text
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            normalize_lyrics_encoding(&[vec![0xfe, 0xff], be].concat()),
            text
        );
        let japanese = "これは日本語の歌詞です。音楽と未来を歌います。".repeat(4);
        let (shift_jis, _, _) = encoding_rs::SHIFT_JIS.encode(&japanese);
        assert_eq!(normalize_lyrics_encoding(&shift_jis), japanese);
        let traditional = "這是繁體中文歌詞，唱著音樂與未來。".repeat(4);
        let (big5, _, _) = encoding_rs::BIG5.encode(&traditional);
        assert_eq!(normalize_lyrics_encoding(&big5), traditional);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn album_download_writes_local_lyrics_and_counts_successes() {
        let root = temp_root();
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri/media-corpus");
        let mp3 = root.join("01.mp3");
        let wav = root.join("02.wav");
        fs::copy(fixtures.join("minimal.mp3"), &mp3).unwrap();
        fs::copy(fixtures.join("minimal.wav"), &wav).unwrap();
        fs::write(mp3.with_extension("lrc"), "MP3 lyrics").unwrap();
        fs::write(wav.with_extension("txt"), "WAV lyrics").unwrap();
        let queue = WriteQueue::default();
        assert_eq!(apply_album_lyrics_at(&root, None, &queue).await, 2);
        assert_eq!(
            read_track_metadata(&mp3).unwrap().lyrics.as_deref(),
            Some("MP3 lyrics")
        );
        assert_eq!(
            read_track_metadata(&wav).unwrap().lyrics.as_deref(),
            Some("WAV lyrics")
        );
        assert!(!queue.is_active());
        assert_eq!(
            download_album_lyrics_at(&root.join("missing"), "http://unused", &queue).await,
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn prefers_synced_and_sends_encoded_optional_query() {
        let (base, request) = server(
            "200 OK",
            r#"{"plainLyrics":"plain","syncedLyrics":"[00:01]synced"}"#,
        );
        assert_eq!(
            fetch_lyrics_at(
                &base,
                "Some Song",
                "Some Artist",
                Some("Album"),
                Some(244.6)
            )
            .await,
            Some("[00:01]synced".to_string())
        );
        let request = request.recv().unwrap();
        assert!(request.contains("GET /api/get?"));
        assert!(request.contains("track_name=Some+Song"));
        assert!(request.contains("artist_name=Some+Artist"));
        assert!(request.contains("album_name=Album"));
        assert!(request.contains("duration=245"));
        assert!(request
            .to_ascii_lowercase()
            .contains(concat!("user-agent: soundrobe/", env!("CARGO_PKG_VERSION"))));
    }

    #[tokio::test]
    async fn falls_back_plain_and_contains_all_failure_modes() {
        let (base, _) = server("200 OK", r#"{"plainLyrics":"plain"}"#);
        assert_eq!(
            fetch_lyrics_at(&base, "Track", "Artist", None, None).await,
            Some("plain".to_string())
        );
        let (empty, _) = server("200 OK", r#"{"instrumental":true}"#);
        assert_eq!(
            fetch_lyrics_at(&empty, "Track", "Artist", None, None).await,
            None
        );
        let (missing, _) = server("404 Not Found", "{}");
        assert_eq!(
            fetch_lyrics_at(&missing, "Track", "Artist", None, None).await,
            None
        );
        let (invalid, _) = server("200 OK", "not-json");
        assert_eq!(
            fetch_lyrics_at(&invalid, "Track", "Artist", None, None).await,
            None
        );
        assert_eq!(
            fetch_lyrics_at("http://unused", "", "Artist", None, None).await,
            None
        );
        assert_eq!(
            fetch_lyrics_at("http://unused", "Track", "", None, None).await,
            None
        );
    }
}
