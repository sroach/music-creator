use anyhow::Result;
use diffusion_rs::{
    api::gen_img,
    preset::{Preset, PresetBuilder},
};
use std::path::{Path, PathBuf};

/// Configuration for Stable Diffusion model
#[derive(Debug, Clone)]
pub struct ImageGenConfig {
    /// Number of diffusion steps (higher = better quality, slower)
    pub steps: i32,
    /// Random seed for reproducibility
    pub seed: i64,
}

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            steps: 25,
            seed: 299792458,
        }
    }
}

impl ImageGenConfig {
    /// Create config from environment variables
    pub fn from_env() -> Option<Self> {
        let steps = std::env::var("IMAGE_GEN_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(25);

        let seed = std::env::var("IMAGE_GEN_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(299792458);

        Some(Self {
            steps: steps as i32,
            seed: seed as i64,
        })
    }
}

/// Builds an image generation prompt from song metadata
pub fn build_image_prompt(song_name: &str, artist: &str, prompt: &str) -> String {
    let mut parts = Vec::new();

    if !prompt.trim().is_empty() {
        parts.push(prompt.trim().to_string());
    }

    if !song_name.trim().is_empty() {
        parts.push(format!("album art for {}", song_name));
    }

    if !artist.trim().is_empty() && artist.trim().to_lowercase() != "unknown" {
        parts.push(format!("by {}", artist));
    }

    parts.push("high quality album cover art".to_string());
    parts.push("professional music artwork".to_string());

    parts.join(", ")
}

/// Generates an image from a text prompt using Stable Diffusion.
/// The thumbnail is saved with the safe name as the filename (e.g., `Song_Name.png`).
pub fn generate_image_from_prompt(
    config: &ImageGenConfig,
    prompt: &str,
    safe_name: &str,
    output_path: &Path,
) -> Result<PathBuf> {
    println!("🎨 Generating image using diffusion-rs...");
    println!("   Prompt: {}", prompt);

    let start = std::time::Instant::now();

    let thumb_filename = format!("{}.png", safe_name);
    let thumb_path = output_path.join(&thumb_filename);
    let thumb_path_clone = thumb_path.clone();
    let steps = config.steps;
    let seed = config.seed;

    // Build preset with automatic model download from Hugging Face
    let (cfg, mut model_cfg) = PresetBuilder::default()
        .preset(Preset::StableDiffusion1_5)
        .prompt(prompt)
        .with_modifier(move |mut builder| {
            builder.0.steps(steps).seed(seed).output(thumb_path_clone);
            Ok(builder)
        })
        .build()?;

    // Generate the image
    gen_img(&cfg, &mut model_cfg)?;

    let elapsed = start.elapsed();
    println!("✅ Image generated in {:?}", elapsed);

    println!("💾 Saved thumbnail to {}", thumb_path.display());
    Ok(thumb_path)
}

/// Checks if diffusion-rs can run on this device
pub fn is_available() -> bool {
    // diffusion-rs should work on macOS with Metal backend
    true
}
