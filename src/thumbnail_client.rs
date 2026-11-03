use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio_stream::StreamExt;

/// Progress events from the Python text-to-image service.
#[derive(Debug, Clone)]
pub enum ThumbnailProgress {
    Queued { message: String },
    Starting { message: String },
    Generating { step: u32, total_steps: u32, percentage: f32 },
    Completed { filename: String },
    Error { message: String },
}

impl std::fmt::Display for ThumbnailProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThumbnailProgress::Queued { message } => write!(f, "Queued: {}", message),
            ThumbnailProgress::Starting { message } => write!(f, "Starting: {}", message),
            ThumbnailProgress::Generating { step, total_steps, percentage } => {
                write!(f, "Generating: step {}/{} ({}%)", step, total_steps, percentage)
            }
            ThumbnailProgress::Completed { filename } => write!(f, "Completed: {}", filename),
            ThumbnailProgress::Error { message } => write!(f, "Error: {}", message),
        }
    }
}

/// Thin HTTP client that calls the external Python text-to-image service.
#[derive(Debug, Clone)]
pub struct ThumbnailClient {
    base_url: String,
    http_client: reqwest::Client,
}

impl ThumbnailClient {
    /// Creates a new ThumbnailClient, reading the service URL from the
    /// `TEXT_TO_IMAGE_URL` environment variable, defaulting to `http://localhost:7070`.
    pub fn new() -> Self {
        let base_url = std::env::var("TEXT_TO_IMAGE_URL")
            .unwrap_or_else(|_| "http://localhost:7070".to_string());

        // Text-to-image generation via SDXL can take 2-5 minutes, so we need
        // very long timeouts. The default reqwest timeout is 30 seconds.
        let http_client = reqwest::ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("reqwest client should build");

        Self {
            base_url,
            http_client,
        }
    }

    /// Generates a thumbnail by calling the Python service's streaming endpoint.
    /// POSTs to `/api/generate-stream` with a JSON body `{ prompt, filename }`,
    /// then consumes the SSE stream, forwarding progress events via the sender.
    /// When completed, downloads the image from `/images/{filename}` and saves it.
    pub async fn generate_and_stream(
        &self,
        prompt: &str,
        filename: &str,
        output_dir: &Path,
        tx: &tokio::sync::mpsc::Sender<ThumbnailProgress>,
    ) -> Result<PathBuf> {
        let url = format!("{}/api/generate-stream", self.base_url);
        println!("🎨 Generating thumbnail via Python service (streaming)...");
        println!("   URL: {}", url);
        println!("   Prompt: {}", prompt);

        let start = std::time::Instant::now();

        let payload = serde_json::json!({
            "prompt": prompt,
            "filename": filename,
        });

        let response = self.http_client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let _ = tx.send(ThumbnailProgress::Error {
                message: format!("Python service returned error {}: {}", status, body),
            }).await;
            anyhow::bail!("Python service returned error {}: {}", status, body);
        }

        // Consume the SSE stream
        let mut buffer = String::new();
        let mut completed_filename: Option<String> = None;

        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Parse SSE events from buffer
            let mut lines: Vec<String> = buffer.split('\n').map(|s| s.to_string()).collect();
            // Keep the last incomplete line in the buffer
            if lines.len() > 1 {
                let last = lines.pop().unwrap();
                buffer = last;
                let complete_lines = lines;

                for line in complete_lines {
                    let trimmed = line.trim();
                    if trimmed.starts_with("data: ") {
                        let data = &trimmed[6..];
                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                            let status = event.get("status").and_then(|s| s.as_str()).unwrap_or("");
                            match status {
                                "queued" => {
                                    let message = event.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                                    let _ = tx.send(ThumbnailProgress::Queued { message }).await;
                                }
                                "starting" => {
                                    let message = event.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                                    let _ = tx.send(ThumbnailProgress::Starting { message }).await;
                                }
                                "generating" => {
                                    let step = event.get("step").and_then(|s| s.as_u64()).unwrap_or(0) as u32;
                                    let total_steps = event.get("total_steps").and_then(|s| s.as_u64()).unwrap_or(0) as u32;
                                    let percentage = event.get("percentage").and_then(|p| p.as_f64()).unwrap_or(0.0) as f32;
                                    let _ = tx.send(ThumbnailProgress::Generating { step, total_steps, percentage }).await;
                                }
                                "completed" => {
                                    let fname = event.get("filename").and_then(|f| f.as_str()).unwrap_or(filename).to_string();
                                    completed_filename = Some(fname.clone());
                                    let _ = tx.send(ThumbnailProgress::Completed { filename: fname }).await;
                                }
                                "error" => {
                                    let message = event.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error").to_string();
                                    let _ = tx.send(ThumbnailProgress::Error { message: message.clone() }).await;
                                    anyhow::bail!("Python service error: {}", message);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Download the completed image from the Python service
        let thumb_filename = format!("{}.png", filename);
        let thumb_path = output_dir.join(&thumb_filename);

        let image_url = format!("{}/images/{}", self.base_url, thumb_filename);
        println!("💾 Downloading thumbnail from {}", image_url);

        let img_response = self.http_client.get(&image_url).send().await?;
        if img_response.status().is_success() {
            let png_bytes = img_response.bytes().await?;
            std::fs::write(&thumb_path, &png_bytes)?;
        } else {
            // Try with the completed filename from the event
            if let Some(ref fname) = completed_filename {
                let image_url = format!("{}/images/{}", self.base_url, fname);
                let img_response = self.http_client.get(&image_url).send().await?;
                if img_response.status().is_success() {
                    let png_bytes = img_response.bytes().await?;
                    std::fs::write(&thumb_path, &png_bytes)?;
                }
            }
        }

        let elapsed = start.elapsed();
        println!("✅ Thumbnail generated in {:?}", elapsed);
        println!("💾 Saved thumbnail to {}", thumb_path.display());

        Ok(thumb_path)
    }

}

/// Builds an image generation prompt from song metadata.
/// Focuses on illustration and text-based artwork for cleaner results.
pub fn build_prompt(song_name: &str, artist: &str, style: &str) -> String {
    let mut parts = Vec::new();

    if !style.trim().is_empty() {
        parts.push(style.trim().to_string());
    }

    if !song_name.trim().is_empty() {
        parts.push(format!("illustration for {}", song_name));
    }

    if !artist.trim().is_empty() && artist.trim().to_lowercase() != "unknown" {
        parts.push(format!("by {}", artist));
    }

    parts.push("digital illustration".to_string());
    parts.push("vector art style".to_string());
    parts.push("clean lines".to_string());
    parts.push("bold colors".to_string());
    parts.push("minimalist design".to_string());
    parts.push("typography".to_string());
    parts.push("text based artwork".to_string());

    parts.join(", ")
}
