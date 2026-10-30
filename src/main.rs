use anyhow::Result;
use axum::{
    body::Body,
    extract::Path,
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
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
    services::ServeDir,
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

    /// Base path prefix for reverse proxy support (e.g. "/music-creator")
    #[arg(long, default_value = "")]
    base_path: String,
}

impl std::fmt::Debug for CliArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliArgs")
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("port", &self.port)
            .field("output_dir", &self.output_dir)
            .field("base_path", &self.base_path)
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

/// Response returned after successful generation
#[derive(Debug, Serialize)]
struct GenerateResponse {
    message: String,
    filename: String,
    size: usize,
    download_url: String,
}

/// Error response
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

/// Single song entry for the list endpoint
#[derive(Debug, Serialize)]
struct SongEntry {
    filename: String,
    song_name: String,
    artist: String,
    created_at: String,
    file_size_bytes: usize,
    download_url: String,
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
    base_path: String,
}

async fn index(
    state: axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    match fs::read_to_string("static/index.html") {
        Ok(content) => {
            // Inject base path into the HTML so static resources resolve correctly
            let content = content.replace(
                "<BASE_PATH_PLACEHOLDER/>",
                &state.base_path,
            );
            Html(content).into_response()
        }
        Err(_) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to load index",
        )
            .into_response(),
    }
}

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
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: "ACESTEP_API_KEY environment variable is not set".to_string(),
            })).into_response();
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

    // Send the request
    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", BASE_URL);

    let response = match client
        .post(&url)
        .headers(api_headers)
        .json(&payload)
        .timeout(Duration::from_secs(600))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: format!("Request failed: {}", e),
            })).into_response();
        }
    };

    let status = response.status();
    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: format!("Failed to read response: {}", e),
            })).into_response();
        }
    };

    if status.as_u16() != 200 {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: format!("API error: {}", status.as_u16()),
        })).into_response();
    }

    // Process SSE streaming response
    let text = String::from_utf8_lossy(&bytes);
    let audio_chunks = extract_audio_chunks(&text);

    if audio_chunks.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: "No audio data found in response".to_string(),
        })).into_response();
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
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
                error: format!("Failed to decode audio: {}", e),
            })).into_response();
        }
    };

    // Sanitize filename
    let safe_name: String = req
        .song_name
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
    fs::create_dir_all(&state.output_dir).unwrap_or_else(|_| ());

    // Save the MP3 file
    let filepath = PathBuf::from(&state.output_dir).join(&filename);
    if let Err(e) = fs::write(&filepath, &audio_bytes) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse {
            error: format!("Failed to save file: {}", e),
        })).into_response();
    }

    // Embed ID3 tags
    if let Err(e) = embed_id3_tags(
        &filepath,
        &req.song_name,
        &req.artist,
        &req.prompt,
        &req.lyrics,
    ) {
        eprintln!("⚠️  Warning: Failed to embed ID3 tags: {}", e);
    } else {
        println!("🏷️  ID3 tags embedded in {}", filename);
    }

    // Save metadata JSON
    let metadata = SongMetadata {
        song_name: req.song_name,
        artist: req.artist,
        prompt: req.prompt,
        lyrics: req.lyrics,
        duration: req.duration,
        bpm: req.bpm,
        model: "acemusic/acestep-v15-turbo".to_string(),
        filename: filename.clone(),
        created_at: Utc::now().to_rfc3339(),
        file_size_bytes: audio_bytes.len(),
    };

    let metadata_path = PathBuf::from(&state.output_dir).join(format!("{}.json", safe_name));
    if let Err(e) = fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap_or_default(),
    ) {
        eprintln!("⚠️  Warning: Failed to save metadata: {}", e);
    }

    // Sync the m3u playlist with the new song
    sync_m3u_playlist(&PathBuf::from(&state.output_dir));

    let download_url = format!("{}/download/{}", state.base_path, filename);

    (StatusCode::OK, Json(GenerateResponse {
        message: "Song generated successfully".to_string(),
        filename,
        size: audio_bytes.len(),
        download_url,
    })).into_response()
}

/// Extract base64 audio chunks from SSE streaming response
fn extract_audio_chunks(sse_text: &str) -> Vec<String> {
    let mut chunks = Vec::new();

    for line in sse_text.lines() {
        // Skip comments
        if line.starts_with(':') {
            continue;
        }

        // Parse SSE data lines
        if let Some(data_str) = line.strip_prefix("data: ") {
            // Check for stream end
            if data_str.trim() == "[DONE]" {
                break;
            }

            // Parse JSON chunk
            let chunk: serde_json::Value = match serde_json::from_str(data_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract audio data from choices
            if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    extract_audio_from_value(choice, &mut chunks);
                }
            }
        }
    }

    chunks
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

/// Sync the m3u playlist file with the current MP3 files in the output directory.
/// Creates or overwrites playlist.m3u with all valid entries sorted by created_at descending.
fn sync_m3u_playlist(output_dir: &PathBuf) {
    let m3u_path = output_dir.join("playlist.m3u");

    let mut songs = Vec::new();

    if let Ok(entries) = fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<SongMetadata>(&content) {
                        let mp3_path = output_dir.join(&metadata.filename);
                        if mp3_path.exists() {
                            songs.push(metadata);
                        }
                    }
                }
            }
        }
    }

    // Sort by created_at descending (newest first)
    songs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    // Build m3u content
    let mut lines = Vec::new();
    lines.push("#EXTM3U".to_string());
    for song in &songs {
        // Duration in seconds
        let duration = song.duration as i64;
        let title = format!("{} - {}", song.artist, song.song_name);
        lines.push(format!("#EXTINF:{},{}", duration, title));
        lines.push(song.filename.clone());
    }

    if let Err(e) = fs::write(&m3u_path, lines.join("\n") + "\n") {
        eprintln!("⚠️  Warning: Failed to write playlist.m3u: {}", e);
    } else {
        println!("🎵 Playlist synced: {} songs in playlist.m3u", songs.len());
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
            headers.insert(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", filename)
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
                            songs.push(SongEntry {
                                filename: metadata.filename.clone(),
                                song_name: metadata.song_name,
                                artist: metadata.artist,
                                created_at: metadata.created_at,
                                file_size_bytes: file_size,
                                download_url: format!("{}/download/{}", state.base_path, metadata.filename),
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

    // Resolve base path from CLI arg or env var
    let base_path = cli.base_path;

    let state = Arc::new(AppState {
        api_key,
        output_dir: output_dir_path.display().to_string(),
        base_path: base_path.clone(),
    });

    // Initialize the m3u playlist on startup if output directory has songs
    sync_m3u_playlist(&output_dir_path);

    println!("🎵 Music Creator starting on http://localhost:{}", port);
    println!("📁 Output directory: {}", state.output_dir);
    if !base_path.is_empty() {
        println!("🔗 Base path: {}", base_path);
    }

    // Build the inner router with all API routes
    let inner = Router::new()
        .route("/", get(index))
        .route("/generate", post(generate))
        .route("/list", get(list_files))
        .route("/download/{filename}", get(download_file))
        .nest_service("/static", ServeDir::new("static"));

    // If a base path is configured, nest the router under it
    let app = if base_path.is_empty() {
        inner
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .with_state(state)
    } else {
        Router::new()
            .nest(&base_path, inner)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .with_state(state)
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
