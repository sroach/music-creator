mod convert;
mod lyrics_gen;
mod thumbnail_client;

use anyhow::Result;
use axum::{
    body::Body,
    extract::Path,
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use convert::{convert_mp3_to_mp4, ConvertErrorResponse, ConvertResponse};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use clap::Parser;
use id3::frame::{Comment, Content, Frame, Lyrics as Id3Lyrics};
use id3::{Tag, TagLike, Version};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tower_http::{
    cors::{Any, CorsLayer},
};

/// Music Creator - Generate songs using ACEMusic AI
#[derive(Parser)]
#[command(name = "music-creator")]
#[command(about = "Generate songs using ACEMusic AI")]
struct CliArgs {
    /// ACEMusic API key (overrides ACESTEP_API_KEY environment variable)
    #[arg(long = "api-key")]
    api_key: Option<String>,

    /// Server port (overrides PORT environment variable)
    #[arg(long, default_value_t = 8001)]
    port: u16,

    /// Output directory for generated MP3 files (defaults to "output")
    #[arg(long, default_value = "output")]
    output_dir: String,
}

impl std::fmt::Debug for CliArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliArgs")
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("port", &self.port)
            .field("output_dir", &self.output_dir)
            .finish()
    }
}

const BASE_URL: &str = "https://api.acemusic.ai";

/// Request payload from the frontend form
#[derive(Debug, Deserialize)]
struct GenerateRequest {
    song_name: String,
    artist: String,
    lyrics: String,
    prompt: String,
    #[serde(default = "default_duration")]
    duration: u32,
    #[serde(default = "default_bpm")]
    bpm: u32,
}

fn default_duration() -> u32 {
    180
}

fn default_bpm() -> u32 {
    120
}


/// Error response
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

/// Error response for thumbnail generation
#[derive(Debug, Serialize)]
struct ThumbnailErrorResponse {
    error: String,
}

/// Request for thumbnail generation with optional user prompt
#[derive(Debug, Deserialize)]
struct ThumbnailRequest {
    #[serde(default)]
    prompt: String,
}

/// Request for lyrics generation
#[derive(Debug, Deserialize)]
struct LyricsGenerationRequest {
    topic: String,
    #[serde(default)]
    style: String,
}

/// Metadata extracted from the lyrics generation output
struct LyricsMetadata {
    duration: u32,
    bpm: u32,
    style: String,
}

/// Parse the lyrics generation output, extracting the metadata block and lyrics text.
/// Returns (Option<LyricsMetadata>, String) where the string is the lyrics without the metadata block.
fn parse_lyrics_output(full_text: &str) -> (Option<LyricsMetadata>, String) {
    let start_marker = "[[METADATA]]";
    let end_marker = "[[END_METADATA]]";

    let start_pos = full_text.find(start_marker);
    let end_pos = full_text.find(end_marker);

    match (start_pos, end_pos) {
        (Some(start), Some(end)) if end > start => {
            let meta_block = &full_text[start + start_marker.len()..end];
            let lyrics = full_text[end + end_marker.len()..].trim().to_string();

            let mut duration: Option<u32> = None;
            let mut bpm: Option<u32> = None;
            let mut style: Option<String> = None;

            for line in meta_block.lines() {
                let line = line.trim();
                if line.starts_with("DURATION:") {
                    if let Some(val) = line.strip_prefix("DURATION:") {
                        duration = val.trim().parse().ok();
                    }
                } else if line.starts_with("BPM:") {
                    if let Some(val) = line.strip_prefix("BPM:") {
                        bpm = val.trim().parse().ok();
                    }
                } else if line.starts_with("STYLE:") {
                    if let Some(val) = line.strip_prefix("STYLE:") {
                        style = Some(val.trim().to_string());
                    }
                }
            }

            let metadata = match (duration, bpm, style) {
                (Some(d), Some(b), Some(s)) => Some(LyricsMetadata {
                    duration: d,
                    bpm: b,
                    style: s,
                }),
                _ => None,
            };

            (metadata, lyrics)
        }
        _ => {
            // No metadata block found, return the full text as lyrics
            (None, full_text.trim().to_string())
        }
    }
}

/// Single song entry for the list endpoint
#[derive(Debug, Serialize)]
struct SongEntry {
    filename: String,
    song_name: String,
    artist: String,
    created_at: String,
    file_size_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    mp4_filename: Option<String>,
    has_thumbnail: bool,
}

/// Response for fetching a single song's details
#[derive(Debug, Serialize)]
struct SongDetails {
    song_name: String,
    artist: String,
    prompt: String,
    lyrics: String,
}

/// Metadata saved alongside the MP3
#[derive(Debug, Serialize, Deserialize)]
struct SongMetadata {
    song_name: String,
    artist: String,
    prompt: String,
    lyrics: String,
    duration: u32,
    bpm: u32,
    model: String,
    filename: String,
    created_at: String,
    file_size_bytes: usize,
}

/// ACEMusic API request payload
#[derive(Debug, Serialize)]
struct AceMusicRequest {
    model: String,
    prompt: String,
    lyrics: String,
    messages: Vec<AceMessage>,
    stream: bool,
    audio_config: AudioConfig,
}

#[derive(Debug, Serialize)]
struct AceMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AudioConfig {
    format: String,
    vocal_language: String,
    duration: u32,
    bpm: u32,
}

/// App state shared across handlers
struct AppState {
    api_key: Option<String>,
    output_dir: String,
    thumbnail_client: thumbnail_client::ThumbnailClient,
    /// Base path prefix for the server (e.g., "/music-creator" behind nginx proxy).
    base_path: String,
}

// Embed static files at compile time so deployment only needs the binary
static INDEX_HTML: &str = include_str!("../static/index.html");
static FAVICON_SVG: &[u8] = include_bytes!("../static/favicon.svg");

async fn index() -> impl axum::response::IntoResponse {
    Html(INDEX_HTML.to_string()).into_response()
}

async fn favicon() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        FAVICON_SVG,
    )
        .into_response()
}

/// SSE handler for song generation endpoint.
/// Streams progress events while the ACEMusic API generates the song.
async fn generate(
    state: axum::extract::State<Arc<AppState>>,
    Json(req): Json<GenerateRequest>,
) -> Response {
    use axum::http::StatusCode;

    // Validate required fields
    if req.lyrics.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "Lyrics are required".to_string(),
        })).into_response();
    }
    if req.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "Style prompt is required".to_string(),
        })).into_response();
    }

    // Check API key
    let api_key = match &state.api_key {
        Some(key) => key.clone(),
        None => {
            let error_json = serde_json::json!({"error": "ACESTEP_API_KEY environment variable is not set"});
            let escaped_json = error_json.to_string()
                .replace("\\", "\\\\")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
            let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
            let done_event = "event: done\ndata: {}\n\n".to_string();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [
                    ("Content-Type", "text/event-stream"),
                    ("Cache-Control", "no-cache"),
                    ("Connection", "keep-alive"),
                    ("X-Accel-Buffering", "no"),
                ],
                axum::body::Body::from(error_event + &done_event),
            ).into_response();
        }
    };

    // Build the API payload
    let payload = AceMusicRequest {
        model: "acemusic/acestep-v15-turbo".to_string(),
        prompt: req.prompt.clone(),
        lyrics: req.lyrics.clone(),
        messages: vec![AceMessage {
            role: "user".to_string(),
            content: req.lyrics.clone(),
        }],
        stream: true,
        audio_config: AudioConfig {
            format: "mp3".to_string(),
            vocal_language: "en".to_string(),
            duration: req.duration,
            bpm: req.bpm,
        },
    };

    // Log the request
    let lyrics_preview: String = req.lyrics.chars().take(100).collect();
    println!("\n============================================================");
    println!("📤 SENDING PAYLOAD TO API");
    println!("============================================================");
    println!("Model: {}", payload.model);
    println!("Prompt (style tags): {}", payload.prompt);
    println!("Lyrics (first 100 chars): {}...", lyrics_preview);
    println!(
        "Audio Config: format=mp3, vocal_language=en, duration={}, bpm={}",
        req.duration, req.bpm
    );
    println!("URL: {}/v1/chat/completions", BASE_URL);
    println!("============================================================\n");

    // Build headers
    let mut api_headers = HeaderMap::new();
    api_headers.insert(
        "Authorization",
        format!("Bearer {}", api_key)
            .parse()
            .expect("Bearer token should parse"),
    );
    api_headers.insert(
        "Content-Type",
        "application/json"
            .parse()
            .expect("Content-Type should parse"),
    );

    // Create an SSE stream channel
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

    let output_dir = state.output_dir.clone();
    let song_name = req.song_name.clone();
    let artist = req.artist.clone();
    let prompt = req.prompt.clone();
    let lyrics = req.lyrics.clone();
    let duration = req.duration;
    let bpm = req.bpm;

    // Spawn a task to run the generation and stream progress
    tokio::spawn(async move {
        // Send started event
        let started_json = serde_json::json!({"message": "Connecting to ACEMusic API..."});
        let escaped_json = started_json.to_string()
            .replace("\\", "\\\\")
            .replace("\n", "\\n")
            .replace("\r", "\\r");
        let started_event = format!("event: started\ndata: {}\n\n", escaped_json);
        if tx.send(started_event).await.is_err() {
            return;
        }

        // Send the request to ACEMusic API with streaming
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let url = format!("{}/v1/chat/completions", BASE_URL);

        let response = match client
            .post(&url)
            .headers(api_headers)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let error_json = serde_json::json!({"error": format!("Request failed: {}", e)});
                let escaped_json = error_json.to_string()
                    .replace("\\", "\\\\")
                    .replace("\n", "\\n")
                    .replace("\r", "\\r");
                let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
                let _ = tx.send(error_event).await;
                return;
            }
        };

        let status = response.status();
        if status.as_u16() != 200 {
            let error_json = serde_json::json!({"error": format!("API error: {}", status.as_u16())});
            let escaped_json = error_json.to_string()
                .replace("\\", "\\\\")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
            let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
            let _ = tx.send(error_event).await;
            return;
        }

        // Send generating event
        let generating_json = serde_json::json!({"message": "Generating audio..."});
        let escaped_json = generating_json.to_string()
            .replace("\\", "\\\\")
            .replace("\n", "\\n")
            .replace("\r", "\\r");
        let generating_event = format!("event: generating\ndata: {}\n\n", escaped_json);
        if tx.send(generating_event).await.is_err() {
            return;
        }

        // Stream the response line-by-line and extract audio chunks
        let mut audio_chunks = Vec::new();
        let mut chunk_count = 0u32;
        let mut buffer = String::new();

        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    buffer.push_str(&text);

                    // Process complete lines
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer.drain(..=newline_pos).collect::<String>();
                        
                        // Skip comments
                        if line.trim_start().starts_with(':') {
                            continue;
                        }

                        // Parse SSE data lines
                        if let Some(data_str) = line.trim_end().strip_prefix("data: ") {
                            // Check for stream end
                            if data_str.trim() == "[DONE]" {
                                break;
                            }

                            // Parse JSON chunk
                            if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data_str) {
                                // Extract audio data from choices
                                if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                                    let mut new_chunks = Vec::new();
                                    for choice in choices {
                                        extract_audio_from_value(choice, &mut new_chunks);
                                    }
                                    if !new_chunks.is_empty() {
                                        chunk_count += 1;
                                        audio_chunks.extend(new_chunks);
                                        
                                        // Estimate progress: each chunk is ~3-5 seconds of audio
                                        // For a 180s song at ~4s per chunk, that's ~45 chunks
                                        let estimated_total = (duration as f32 / 4.0).ceil() as u32;
                                        let percentage = ((chunk_count as f32 / estimated_total as f32) * 100.0).min(95.0) as u32;
                                        
                                        let progress_json = serde_json::json!({
                                            "chunk": chunk_count,
                                            "percentage": percentage
                                        });
                                        let escaped_json = progress_json.to_string()
                                            .replace("\\", "\\\\")
                                            .replace("\n", "\\n")
                                            .replace("\r", "\\r");
                                        let progress_event = format!("event: progress\ndata: {}\n\n", escaped_json);
                                        if tx.send(progress_event).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let error_json = serde_json::json!({"error": format!("Failed to read response: {}", e)});
                    let escaped_json = error_json.to_string()
                        .replace("\\", "\\\\")
                        .replace("\n", "\\n")
                        .replace("\r", "\\r");
                    let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
                    let _ = tx.send(error_event).await;
                    return;
                }
            }
        }

        if audio_chunks.is_empty() {
            let error_json = serde_json::json!({"error": "No audio data found in response"});
            let escaped_json = error_json.to_string()
                .replace("\\", "\\\\")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
            let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
            let _ = tx.send(error_event).await;
            return;
        }

        // Combine and decode audio
        let combined_audio = audio_chunks.join("");
        let clean_audio = if combined_audio.contains(',') {
            combined_audio
                .splitn(2, ',')
                .nth(1)
                .unwrap_or(&combined_audio)
                .to_string()
        } else {
            combined_audio
        };

        let audio_bytes = match BASE64.decode(&clean_audio) {
            Ok(b) => b,
            Err(e) => {
                let error_json = serde_json::json!({"error": format!("Failed to decode audio: {}", e)});
                let escaped_json = error_json.to_string()
                    .replace("\\", "\\\\")
                    .replace("\n", "\\n")
                    .replace("\r", "\\r");
                let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
                let _ = tx.send(error_event).await;
                return;
            }
        };

        // Sanitize filename
        let safe_name: String = song_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe_name = safe_name.trim().to_string();
        let safe_name = if safe_name.is_empty() {
            "generated_song".to_string()
        } else {
            safe_name.replace(' ', "_")
        };
        let filename = format!("{}.mp3", safe_name);

        // Ensure output directory exists
        fs::create_dir_all(&output_dir).unwrap_or_else(|_| ());

        // Save the MP3 file
        let filepath = PathBuf::from(&output_dir).join(&filename);
        if let Err(e) = fs::write(&filepath, &audio_bytes) {
            let error_json = serde_json::json!({"error": format!("Failed to save file: {}", e)});
            let escaped_json = error_json.to_string()
                .replace("\\", "\\\\")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
            let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
            let _ = tx.send(error_event).await;
            return;
        }

        // Embed ID3 tags
        if let Err(e) = embed_id3_tags(
            &filepath,
            &song_name,
            &artist,
            &prompt,
            &lyrics,
        ) {
            eprintln!("⚠️  Warning: Failed to embed ID3 tags: {}", e);
        } else {
            println!("🏷️  ID3 tags embedded in {}", filename);
        }

        // Save metadata JSON
        let metadata = SongMetadata {
            song_name,
            artist,
            prompt,
            lyrics,
            duration,
            bpm,
            model: "acemusic/acestep-v15-turbo".to_string(),
            filename: filename.clone(),
            created_at: Utc::now().to_rfc3339(),
            file_size_bytes: audio_bytes.len(),
        };

        let metadata_path = PathBuf::from(&output_dir).join(format!("{}.json", safe_name));
        if let Err(e) = fs::write(
            &metadata_path,
            serde_json::to_string_pretty(&metadata).unwrap_or_default(),
        ) {
            eprintln!("⚠️  Warning: Failed to save metadata: {}", e);
        }

        // Sync the m3u playlist with the new song
        sync_m3u_playlist(&PathBuf::from(&output_dir));

        // Send done event
        let done_json = serde_json::json!({
            "message": "Song generated successfully",
            "filename": filename,
            "size": audio_bytes.len()
        });
        let escaped_json = done_json.to_string()
            .replace("\\", "\\\\")
            .replace("\n", "\\n")
            .replace("\r", "\\r");
        let done_event = format!("event: done\ndata: {}\n\n", escaped_json);
        let _ = tx.send(done_event).await;
    });

    // Build the SSE response body from the channel receiver
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_stream::StreamExt;
    let receiver_stream = ReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(receiver_stream.map(|msg| {
        Ok::<_, std::convert::Infallible>(msg)
    }));

    (
        StatusCode::OK,
        [
            ("Content-Type", "text/event-stream"),
            ("Cache-Control", "no-cache"),
            ("Connection", "keep-alive"),
            ("X-Accel-Buffering", "no"),
        ],
        body,
    )
        .into_response()
}


/// Recursively extract audio data from a JSON value
fn extract_audio_from_value(value: &serde_json::Value, chunks: &mut Vec<String>) {
    // Check for "audio" field
    if let Some(audio) = value.get("audio") {
        match audio {
            serde_json::Value::String(s) => {
                if !s.is_empty() {
                    chunks.push(s.clone());
                }
            }
            serde_json::Value::Object(obj) => {
                if let Some(data) = obj.get("data").and_then(|d| d.as_str()) {
                    if !data.is_empty() {
                        chunks.push(data.to_string());
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let serde_json::Value::Object(obj) = item {
                        // Check for audio_url
                        if let Some(audio_url) = obj.get("audio_url") {
                            if let Some(url) = audio_url.get("url").and_then(|u| u.as_str()) {
                                if !url.is_empty() {
                                    chunks.push(url.to_string());
                                    continue;
                                }
                            }
                        }
                        // Check for data field
                        if let Some(data) = obj.get("data").and_then(|d| d.as_str()) {
                            if !data.is_empty() {
                                chunks.push(data.to_string());
                            }
                        }
                    } else if let serde_json::Value::String(s) = item {
                        if !s.is_empty() {
                            chunks.push(s.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Recurse into delta and message objects
    for key in &["delta", "message"] {
        if let Some(sub) = value.get(key) {
            extract_audio_from_value(sub, chunks);
        }
    }
}

/// Embed ID3 tags into the MP3 file
fn embed_id3_tags(
    filepath: &PathBuf,
    title: &str,
    artist: &str,
    comment: &str,
    lyrics: &str,
) -> Result<()> {
    let mut tag = match Tag::read_from_path(filepath) {
        Ok(t) => t,
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => Tag::new(),
        Err(e) => return Err(e.into()),
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());

    // Add comment frame (COMM) - truncate to 300 chars
    let comment_text: String = comment.chars().take(300).collect();
    let comment_frame = Frame::with_content(
        "COMM",
        Content::Comment(Comment {
            lang: "eng".to_string(),
            description: "Style Prompt".to_string(),
            text: comment_text,
        }),
    );
    tag.add_frame(comment_frame);

    // Add unsynchronized lyrics frame (USLT)
    let lyrics_frame = Frame::with_content(
        "USLT",
        Content::Lyrics(Id3Lyrics {
            lang: "eng".to_string(),
            description: "Lyrics".to_string(),
            text: lyrics.to_string(),
        }),
    );
    tag.add_frame(lyrics_frame);

    tag.write_to_path(filepath, Version::Id3v24)?;
    Ok(())
}

/// Build M3U playlist content for all MP4 videos in the output directory.
/// Uses HTTP URLs so the playlist can be imported into Apple TV apps (IPTV Player, Infuse, VLC).
fn build_mp4_playlist_content(output_dir: &PathBuf, base_url: &str) -> String {
    if !output_dir.exists() {
        return "#EXTM3U\n".to_string();
    }

    // Collect MP4 filenames from JSON metadata files
    let mut entries: Vec<(String, String, String)> = Vec::new(); // (song_name, artist, mp4_filename)

    if let Ok(dir_entries) = fs::read_dir(output_dir) {
        for entry in dir_entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<SongMetadata>(&content) {
                        let safe_name = metadata.filename.trim_end_matches(".mp3");
                        let mp4_filename = format!("{}.mp4", safe_name);
                        let mp4_path = output_dir.join(&mp4_filename);
                        if mp4_path.exists() {
                            entries.push((
                                metadata.song_name.clone(),
                                metadata.artist.clone(),
                                mp4_filename,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Sort by filename (alphabetically by song)
    entries.sort_by(|a, b| a.2.cmp(&b.2));

    // Build M3U content with #EXTM3U header and #EXTINF metadata
    let mut lines = Vec::new();
    lines.push("#EXTM3U".to_string());
    for (song_name, artist, mp4_filename) in &entries {
        let title = if artist != "Unknown" && !artist.is_empty() {
            format!("{} - {}", artist, song_name)
        } else {
            song_name.clone()
        };
        let url = format!("{}/download/{}", base_url.trim_end_matches('/'), mp4_filename);
        lines.push(format!("#EXTINF:-1,{}", title));
        lines.push(url);
    }

    lines.join("\n") + "\n"
}

/// Serve the MP4 M3U playlist for download/import.
/// Prefers the `base_url` query parameter (from the browser's current context) so that
/// host, protocol, and base path always match what the user sees. Falls back to server
/// headers + config when the parameter is absent.
async fn get_mp4_playlist(
    state: axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    use axum::http::StatusCode;

    // Prefer the browser-provided base URL; fall back to server headers + config
    let base_url = query.get("base_url")
        .cloned()
        .unwrap_or_else(|| {
            let scheme = headers.get("X-Forwarded-Proto").and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            let host = headers.get("X-Forwarded-Host").and_then(|v| v.to_str().ok())
                .or_else(|| headers.get("Host").and_then(|v| v.to_str().ok()))
                .unwrap_or("localhost:8001");
            format!("{}://{}{}", scheme, host, state.base_path)
        });

    let output_dir = PathBuf::from(&state.output_dir);
    let contents = build_mp4_playlist_content(&output_dir, &base_url);

    // Also save to disk for local access
    let m3u_path = output_dir.join("playlist-mp4.m3u");
    if let Err(e) = fs::write(&m3u_path, &contents) {
        eprintln!("⚠️  Warning: Failed to write playlist-mp4.m3u: {}", e);
    }

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "Content-Type",
        "audio/x-mpegurl"
            .parse()
            .expect("Content-Type should parse"),
    );
    response_headers.insert(
        "Content-Disposition",
        "attachment; filename=\"playlist-mp4.m3u\""
            .parse()
            .expect("Content-Disposition should parse"),
    );
    (StatusCode::OK, response_headers, Body::from(contents)).into_response()
}

/// Sync the m3u playlist file with all MP3 files in the output directory.
/// Generates a simple m3u format (one absolute path per line) for Liquidsoap compatibility.
/// No #EXTM3U header or #EXTINF metadata lines — just raw paths.
fn sync_m3u_playlist(output_dir: &PathBuf) {
    let m3u_path = output_dir.join("playlist.m3u");

    // Get the absolute path of the output directory
    let abs_output_dir = match std::fs::canonicalize(output_dir) {
        Ok(p) => p,
        Err(_) => output_dir.clone(),
    };

    // Collect filenames from JSON metadata files
    let mut filenames: Vec<String> = Vec::new();
    let mut known_filenames = std::collections::HashSet::new();

    if let Ok(entries) = fs::read_dir(&abs_output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<SongMetadata>(&content) {
                        let mp3_path = abs_output_dir.join(&metadata.filename);
                        if mp3_path.exists() {
                            known_filenames.insert(metadata.filename.clone());
                            filenames.push(metadata.filename.clone());
                        }
                    }
                }
            }
        }
    }

    // Find orphan MP3s (no matching JSON metadata)
    let mut orphan_filenames: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&abs_output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("mp3") {
                let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
                if filename.is_empty() || filename == "playlist.m3u" {
                    continue;
                }
                if known_filenames.contains(&filename) {
                    continue;
                }
                orphan_filenames.push(filename);
            }
        }
    }

    // Append orphans after metadata-backed songs
    filenames.extend(orphan_filenames);

    // Build simple m3u content — one absolute path per line, no metadata
    let mut lines = Vec::new();
    for filename in &filenames {
        let abs_path = abs_output_dir.join(filename);
        if let Some(path_str) = abs_path.to_str() {
            lines.push(path_str.to_string());
        }
    }

    if let Err(e) = fs::write(&m3u_path, lines.join("\n") + "\n") {
        eprintln!("⚠️  Warning: Failed to write playlist.m3u: {}", e);
    } else {
        println!("🎵 Playlist synced: {} songs in playlist.m3u", filenames.len());
    }
}

async fn download_file(
    state: axum::extract::State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> Response {
    use axum::http::StatusCode;

    let filepath = PathBuf::from(&state.output_dir).join(&filename);

    if !filepath.exists() {
        return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "File not found".to_string(),
        })).into_response();
    }

    match fs::read(&filepath) {
        Ok(contents) => {
            let mut headers = HeaderMap::new();
            // Determine content type based on file extension
            let content_type = if filename.ends_with(".mp4") {
                "video/mp4"
            } else if filename.ends_with(".png") {
                "image/png"
            } else {
                "audio/mpeg"
            };
            headers.insert(
                "Content-Type",
                content_type
                    .parse()
                    .expect("Content-Type should parse"),
            );
            headers.insert(
                "Content-Disposition",
                format!("inline; filename=\"{}\"", filename)
                    .parse()
                    .expect("Content-Disposition should parse"),
            );

            (StatusCode::OK, headers, Body::from(contents)).into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "Failed to read file".to_string(),
        })).into_response(),
    }
}

async fn list_files(
    state: axum::extract::State<Arc<AppState>>,
) -> Response {
    use axum::http::StatusCode;

    let output_dir = PathBuf::from(&state.output_dir);
    
    if !output_dir.exists() {
        return (StatusCode::OK, Json(Vec::<SongEntry>::new())).into_response();
    }

    let mut songs = Vec::new();

    if let Ok(entries) = fs::read_dir(&output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<SongMetadata>(&content) {
                        // Check if the corresponding MP3 exists
                        let mp3_path = output_dir.join(&metadata.filename);
                        if mp3_path.exists() {
                            let file_size = mp3_path.metadata().ok().map(|m| m.len() as usize).unwrap_or(metadata.file_size_bytes);

                            // Check if an MP4 conversion exists
                            let safe_name = metadata.filename.trim_end_matches(".mp3");
                            let mp4_filename = format!("{}.mp4", safe_name);
                            let mp4_exists = output_dir.join(&mp4_filename).exists();
                            let mp4_filename_field = if mp4_exists {
                                Some(mp4_filename)
                            } else {
                                None
                            };

                            // Check if a thumbnail exists for this song
                            let thumbnail_path = output_dir.join(format!("{}.png", safe_name));
                            let has_thumbnail = thumbnail_path.exists();

                            songs.push(SongEntry {
                                filename: metadata.filename.clone(),
                                song_name: metadata.song_name,
                                artist: metadata.artist,
                                created_at: metadata.created_at,
                                file_size_bytes: file_size,
                                mp4_filename: mp4_filename_field,
                                has_thumbnail,
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by created_at descending (newest first)
    songs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    (StatusCode::OK, Json(songs)).into_response()
}

/// Fetch full details (prompt, lyrics) for a single song from its JSON metadata
async fn get_song_details(
    state: axum::extract::State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> Response {
    use axum::http::StatusCode;

    let output_dir = PathBuf::from(&state.output_dir);
    let json_path = output_dir.join(filename).with_extension("json");

    if !json_path.exists() {
        return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: "Song metadata not found".to_string(),
        })).into_response();
    }

    match fs::read_to_string(&json_path) {
        Ok(content) => {
            match serde_json::from_str::<SongMetadata>(&content) {
                Ok(metadata) => {
                    (StatusCode::OK, Json(SongDetails {
                        song_name: metadata.song_name,
                        artist: metadata.artist,
                        prompt: metadata.prompt,
                        lyrics: metadata.lyrics,
                    })).into_response()
                }
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                    error: "Failed to parse metadata".to_string(),
                })).into_response(),
            }
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "Failed to read metadata file".to_string(),
        })).into_response(),
    }
}

/// SSE handler for thumbnail generation endpoint.
/// Streams progress events while the Python text-to-image service generates the thumbnail.
/// Accepts an optional JSON body with a user-provided prompt.
async fn generate_thumbnail_endpoint(
    state: axum::extract::State<Arc<AppState>>,
    Path(filename): Path<String>,
    body: axum::extract::Json<Option<ThumbnailRequest>>,
) -> Response {
    use axum::http::StatusCode;

    let output_dir = PathBuf::from(&state.output_dir);
    let mp3_path = output_dir.join(&filename);

    if !mp3_path.exists() {
        return (StatusCode::NOT_FOUND, Json(ThumbnailErrorResponse {
            error: "MP3 file not found".to_string(),
        })).into_response();
    }

    // Load song metadata
    let safe_name = filename.trim_end_matches(".mp3").to_string();
    let metadata_path = output_dir.join(format!("{}.json", safe_name));
    let (song_name, artist, style) = if let Ok(content) = fs::read_to_string(&metadata_path) {
        if let Ok(metadata) = serde_json::from_str::<SongMetadata>(&content) {
            (metadata.song_name, metadata.artist, metadata.prompt)
        } else {
            (safe_name.clone(), "Unknown".to_string(), String::new())
        }
    } else {
        (safe_name.clone(), "Unknown".to_string(), String::new())
    };

    // Check if thumbnail already exists
    let thumbnail_path = output_dir.join(format!("{}.png", safe_name));
    if thumbnail_path.exists() {
        let thumb_filename = format!("{}.png", safe_name);
        let escaped_json = serde_json::json!({"thumbnail_filename": thumb_filename}).to_string();
        let existing_event = format!("event: existing\ndata: {}\n\n", escaped_json);
        let done_event = "event: done\ndata: {}\n\n".to_string();
        return (
            StatusCode::OK,
            [
                ("Content-Type", "text/event-stream"),
                ("Cache-Control", "no-cache"),
                ("Connection", "keep-alive"),
                ("X-Accel-Buffering", "no"),
            ],
            axum::body::Body::from(existing_event + &done_event),
        ).into_response();
    }

    // Create an SSE stream channel
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

    let client = state.thumbnail_client.clone();
    let output_dir_clone = output_dir.clone();
    let safe_name_for_task = safe_name.clone();
    let song_name_for_task = song_name.clone();
    let artist_for_task = artist.clone();
    let style_for_task = style.clone();

    let user_prompt = body.0.and_then(|r| if r.prompt.is_empty() { None } else { Some(r.prompt) });

    // Spawn a task to run the generation and stream progress from the Python service
    tokio::spawn(async move {
        let prompt = if let Some(user_prompt) = user_prompt {
            user_prompt
        } else {
            thumbnail_client::build_prompt(&song_name_for_task, &artist_for_task, &style_for_task)
        };

        // Create a channel for progress events from the client
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<thumbnail_client::ThumbnailProgress>(32);

        // Spawn the actual generation task
        let gen_client = client.clone();
        let gen_output_dir = output_dir_clone.clone();
        let gen_safe_name = safe_name_for_task.clone();
        let mut gen_task = tokio::spawn(async move {
            gen_client.generate_and_stream(&prompt, &gen_safe_name, &gen_output_dir, &progress_tx).await
        });

        // Forward progress events from the Python service to the frontend SSE stream
        let mut gen_result = None;
        loop {
            tokio::select! {
                result = &mut gen_task => {
                    gen_result = Some(result);
                    break;
                }
                Some(progress) = progress_rx.recv() => {
                    let sse_event = match &progress {
                        thumbnail_client::ThumbnailProgress::Queued { message } => {
                            let json = serde_json::json!({"message": message});
                            let escaped = json.to_string().replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
                            format!("event: started\ndata: {}\n\n", escaped)
                        }
                        thumbnail_client::ThumbnailProgress::Starting { message } => {
                            let json = serde_json::json!({"message": message});
                            let escaped = json.to_string().replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
                            format!("event: generating\ndata: {}\n\n", escaped)
                        }
                        thumbnail_client::ThumbnailProgress::Generating { step, total_steps, percentage } => {
                            let json = serde_json::json!({"step": step, "total_steps": total_steps, "percentage": percentage});
                            let escaped = json.to_string().replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
                            format!("event: progress\ndata: {}\n\n", escaped)
                        }
                        thumbnail_client::ThumbnailProgress::Completed { filename } => {
                            let json = serde_json::json!({"thumbnail_filename": filename});
                            let escaped = json.to_string().replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
                            format!("event: done\ndata: {}\n\n", escaped)
                        }
                        thumbnail_client::ThumbnailProgress::Error { message } => {
                            let json = serde_json::json!({"error": message});
                            let escaped = json.to_string().replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
                            format!("event: error\ndata: {}\n\n", escaped)
                        }
                    };
                    if tx.send(sse_event).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }

        // If we exited the loop without a done/error event, handle the final result
        if let Some(result) = gen_result {
            match result {
                Ok(Ok(_)) => {
                    // Already sent done event via progress stream
                }
                Ok(Err(e)) => {
                    let error_json = serde_json::json!({"error": e.to_string()});
                    let escaped_json = error_json.to_string()
                        .replace("\\", "\\\\")
                        .replace("\n", "\\n")
                        .replace("\r", "\\r");
                    let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
                    let _ = tx.send(error_event).await;
                }
                Err(e) => {
                    let error_json = serde_json::json!({"error": e.to_string()});
                    let escaped_json = error_json.to_string()
                        .replace("\\", "\\\\")
                        .replace("\n", "\\n")
                        .replace("\r", "\\r");
                    let error_event = format!("event: error\ndata: {}\n\n", escaped_json);
                    let _ = tx.send(error_event).await;
                }
            }
        }
    });

    // Build the SSE response body from the channel receiver
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_stream::StreamExt;
    let receiver_stream = ReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(receiver_stream.map(|msg| {
        Ok::<_, std::convert::Infallible>(msg)
    }));

    (
        StatusCode::OK,
        [
            ("Content-Type", "text/event-stream"),
            ("Cache-Control", "no-cache"),
            ("Connection", "keep-alive"),
            ("X-Accel-Buffering", "no"),
        ],
        body,
    )
        .into_response()
}

/// Request body for video conversion.
#[derive(Debug, serde::Deserialize)]
struct ConvertRequest {
    /// Optional song title to overlay on the video.
    #[serde(default)]
    title: Option<String>,
}

async fn convert_to_video(
    state: axum::extract::State<Arc<AppState>>,
    Path(filename): Path<String>,
    Json(req): Json<ConvertRequest>,
) -> Response {
    use axum::http::StatusCode;

    let output_dir = PathBuf::from(&state.output_dir);
    let mp3_path = output_dir.join(&filename);

    if !mp3_path.exists() {
        return (StatusCode::NOT_FOUND, Json(ConvertErrorResponse {
            error: "MP3 file not found".to_string(),
        })).into_response();
    }

    // Convert MP3 to MP4 using FFmpeg with waveform visualization and optional title
    let title = req.title.filter(|t| !t.trim().is_empty());
    match convert_mp3_to_mp4(&mp3_path, &output_dir, title.as_deref()) {
        Ok(mp4_filename) => {
            // Sync the MP4 playlist on disk with a localhost placeholder
            // (the endpoint regenerates with the correct URL on download)
            let playlist_content = build_mp4_playlist_content(&output_dir, &format!("http://localhost:8001{}", state.base_path));
            let m3u_path = output_dir.join("playlist-mp4.m3u");
            let _ = fs::write(&m3u_path, &playlist_content);
            (StatusCode::OK, Json(ConvertResponse {
                message: "Video created successfully".to_string(),
                mp4_filename,
            })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ConvertErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}

/// SSE handler for lyrics generation endpoint
async fn generate_lyrics(
    Json(req): Json<LyricsGenerationRequest>,
) -> Response {
    use axum::http::StatusCode;
    use foundation_models::StreamEvent;

    // Validate the topic
    if req.topic.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: "Topic is required".to_string(),
        })).into_response();
    }

    // Check model availability
    if !lyrics_gen::is_available() {
        let reason = lyrics_gen::availability_reason();
        return (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse {
            error: format!("Foundation Models unavailable: {}", reason),
        })).into_response();
    }

    // Create the lyrics generator
    let style = if req.style.trim().is_empty() {
        "pop, male vocal".to_string()
    } else {
        req.style.clone()
    };

    let generator = match lyrics_gen::LyricsGenerator::new(&req.topic, &style) {
        Ok(g) => g,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: format!("Failed to create generation session: {}", e),
            })).into_response();
        }
    };

    // Log the request
    println!("\n============================================================");
    println!("📝 GENERATING LYRICS");
    println!("============================================================");
    println!("Topic: {}", req.topic);
    println!("Style: {}", style);
    println!("============================================================\n");

    // Create an SSE stream channel
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

    // Spawn a task to run the blocking stream on a thread
    tokio::task::spawn_blocking(move || {
        use std::sync::mpsc;

        // Collect all chunks using a channel so the closure can be 'static
        let (chunk_tx, chunk_rx) = mpsc::channel::<String>();
        let error_tx = chunk_tx.clone();

        let result = generator.stream("Generate the lyrics now.", move |event| {
            match event {
                StreamEvent::Chunk(text) => {
                    let _ = chunk_tx.send(text.to_string());
                }
                StreamEvent::Replace(text) => {
                    let _ = chunk_tx.send(text.to_string());
                }
                StreamEvent::Error(e) => {
                    let _ = error_tx.send(format!("__ERROR__{}", e));
                }
                _ => {}
            }
        });

        // chunk_tx and error_tx were moved into the closure and dropped when stream() returned

        // Collect all chunks
        let mut full_text = String::new();
        let mut error_msg: Option<String> = None;
        for msg in chunk_rx.iter() {
            if let Some(err) = msg.strip_prefix("__ERROR__") {
                error_msg = Some(err.to_string());
            } else {
                full_text.push_str(&msg);
            }
        }

        if let Err(e) = result {
            error_msg.get_or_insert_with(|| e.to_string());
        }

        // Parse metadata and lyrics from the full text
        let (metadata, lyrics_text) = parse_lyrics_output(&full_text);

        // Send metadata event if we found any
        if let Some(meta) = metadata {
            let meta_json = serde_json::json!({
                "duration": meta.duration,
                "bpm": meta.bpm,
                "style": meta.style
            });
            let escaped_json = meta_json.to_string()
                .replace("\\", "\\\\")
                .replace("\n", "\\n")
                .replace("\r", "\\r");
            let meta_event = format!("event: metadata\ndata: {}\n\n", escaped_json);
            if tx.blocking_send(meta_event).is_err() {
                return;
            }
        }

        // Stream the lyrics text in chunks for real-time display
        let escaped_lyrics = lyrics_text.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r");
        // Split into smaller chunks for streaming effect
        let chunk_size = 20; // characters per chunk
        let chars: Vec<char> = escaped_lyrics.chars().collect();
        for i in (0..chars.len()).step_by(chunk_size) {
            let end = (i + chunk_size).min(chars.len());
            let chunk: String = chars[i..end].iter().collect();
            let msg = format!("data: {}\n\n", chunk);
            if tx.blocking_send(msg).is_err() {
                break;
            }
            // Small delay to simulate streaming
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Send error if there was one
        if let Some(err) = error_msg {
            let err_event = format!("event: error\ndata: {{\"error\": \"{}\"}}\n\n", err);
            let _ = tx.blocking_send(err_event);
        }

        // Send done event
        let done_event = "event: done\ndata: {}\n\n".to_string();
        let _ = tx.blocking_send(done_event);

        // Drop the sender so the receiver stream ends
        drop(tx);
    });

    // Build the SSE response body from the channel receiver
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_stream::StreamExt;
    let receiver_stream = ReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(receiver_stream.map(|msg| {
        Ok::<_, std::convert::Infallible>(msg)
    }));

    (
        StatusCode::OK,
        [
            ("Content-Type", "text/event-stream"),
            ("Cache-Control", "no-cache"),
            ("Connection", "keep-alive"),
            ("X-Accel-Buffering", "no"),
        ],
        body,
    )
        .into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = CliArgs::parse();

    // Load .env file if it exists (lowest priority)
    let _ = dotenvy::dotenv();

    // API key priority: CLI arg > env var > .env file
    let api_key = cli
        .api_key
        .or_else(|| std::env::var("ACESTEP_API_KEY").ok());

    // Port priority: CLI arg > env var > .env file > default
    let port = cli.port;

    // Resolve output directory relative to the project root (executable's parent)
    // rather than the current working directory, so it works regardless of where
    // you run the binary from.
    let project_root = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        // In debug builds, the exe is in target/debug/, so go up two levels to reach project root
        .and_then(|p| {
            if p.components().any(|c| c.as_os_str() == "debug" || c.as_os_str() == "release") {
                // p is target/debug/, go up to target/, then up again to project root
                p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf())
            } else {
                Some(p)
            }
        })
        // Fallback to current directory if we can't determine the project root
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let output_dir_path = project_root.join(&cli.output_dir);

    // Ensure output directory exists
    fs::create_dir_all(&output_dir_path).unwrap_or_else(|_| ());

    // Base path prefix for proxy deployment (e.g., "/music-creator")
    let base_path = std::env::var("MUSIC_CREATOR_BASE_PATH").unwrap_or_default();

    let state = Arc::new(AppState {
        api_key,
        output_dir: output_dir_path.display().to_string(),
        thumbnail_client: thumbnail_client::ThumbnailClient::new(),
        base_path,
    });

    // Initialize the m3u playlist on startup if output directory has songs
    sync_m3u_playlist(&output_dir_path);

    println!("🎵 Music Creator starting on http://localhost:{}", port);
    println!("📁 Output directory: {}", state.output_dir);

    // Log Foundation Models availability
    if lyrics_gen::is_available() {
        println!("✅ Foundation Models is available for lyrics generation");
    } else {
        println!("⚠️  Foundation Models is NOT available: {}", lyrics_gen::availability_reason());
        println!("   Lyrics generation will not work. Enable Apple Intelligence in System Settings.");
    }

    // Build the router with all API routes
    let app = Router::new()
        .route("/", get(index))
        .route("/generate", post(generate))
        .route("/generate-lyrics", post(generate_lyrics))
        .route("/list", get(list_files))
        .route("/song/{filename}", get(get_song_details))
        .route("/download/{filename}", get(download_file))
        .route("/thumbnail/{filename}", post(generate_thumbnail_endpoint))
        .route("/convert/{filename}", post(convert_to_video))
        .route("/playlist-mp4.m3u", get(get_mp4_playlist))
        .route("/static/favicon.svg", get(favicon))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
