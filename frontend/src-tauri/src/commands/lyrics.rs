//! LRCLIB lyrics fetch command.

use serde::Deserialize;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://lrclib.net/api";
const USER_AGENT: &str = "auto-tagger/0.1.0 (https://github.com/auto-tagger)";

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
    use std::sync::mpsc;
    use std::thread;

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
            .contains("user-agent: auto-tagger/0.1.0"));
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
