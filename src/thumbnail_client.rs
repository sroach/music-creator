use anyhow::Result;
use std::path::{Path, PathBuf};

/// Thin HTTP client that calls the external Python text-to-image service.
#[derive(Debug, Clone)]
pub struct ThumbnailClient {
    base_url: String,
}

impl ThumbnailClient {
    /// Creates a new ThumbnailClient, reading the service URL from the
    /// `TEXT_TO_IMAGE_URL` environment variable, defaulting to `http://localhost:7070`.
    pub fn new() -> Self {
        let base_url = std::env::var("TEXT_TO_IMAGE_URL")
            .unwrap_or_else(|_| "http://localhost:7070".to_string());
        Self { base_url }
    }

    /// Generates a thumbnail by calling the Python service.
    /// POSTs to `/api/generate` with a JSON body `{ prompt, filename }`,
    /// then saves the PNG response to disk in the output directory.
    pub async fn generate(
        &self,
        prompt: &str,
        filename: &str,
        output_dir: &Path,
    ) -> Result<PathBuf> {
        let url = format!("{}/api/generate", self.base_url);
        println!("🎨 Generating thumbnail via Python service...");
        println!("   URL: {}", url);
        println!("   Prompt: {}", prompt);

        let start = std::time::Instant::now();

        let payload = serde_json::json!({
            "prompt": prompt,
            "filename": filename,
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Python service returned error {}: {}", status, body);
        }

        let png_bytes = response.bytes().await?;

        let thumb_filename = format!("{}.png", filename);
        let thumb_path = output_dir.join(&thumb_filename);

        std::fs::write(&thumb_path, &png_bytes)?;

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
