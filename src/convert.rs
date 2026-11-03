use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Serialize)]
pub struct ConvertResponse {
    pub message: String,
    pub mp4_filename: String,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ConvertErrorResponse {
    pub error: String,
}


/// Converts an MP3 file to an MP4 video using FFmpeg.
///
/// Uses a static thumbnail image (generated or default) and the MP3 audio
/// to create an H.264/AAC MP4 suitable for YouTube upload.
pub fn convert_mp3_to_mp4(
    mp3_path: &Path,
    output_dir: &Path,
    thumbnail_path: &Path,
) -> Result<String> {
    // Derive output filename from input
    let mp4_filename = mp3_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string()
        + ".mp4";

    let mp4_path = output_dir.join(&mp4_filename);

    // Check if ffmpeg is available
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    match ffmpeg_check {
        Ok(output) if output.status.success() => {
            // FFmpeg is available, proceed
        }
        _ => {
            anyhow::bail!(
                "FFmpeg is not installed or not in PATH. Install it via 'brew install ffmpeg' on macOS."
            );
        }
    }

    // Run FFmpeg to convert MP3 + thumbnail to MP4
    let status = Command::new("ffmpeg")
        .args([
            "-y", // Overwrite output file
            "-loop", "1", // Loop the image
            "-i", thumbnail_path.to_str().unwrap(), // Input image
            "-i", mp3_path.to_str().unwrap(), // Input audio
            "-shortest", // Stop when audio ends
            "-c:v", "libx264", // H.264 video codec
            "-c:a", "aac", // AAC audio codec
            "-pix_fmt", "yuv420p", // Required for YouTube compatibility
            "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", // Ensure even dimensions
            "-movflags", "+faststart", // Optimize for web streaming
            mp4_path.to_str().unwrap(),
        ])
        .output()?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("FFmpeg failed: {}", stderr);
    }

    println!("🎬 Converted {} to {}", mp3_path.display(), mp4_path.display());
    Ok(mp4_filename)
}


