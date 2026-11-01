use anyhow::Result;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;


/// Response after successful conversion.
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

/// Generates a thumbnail image from song metadata and saves it as PNG.
/// Returns the path to the generated thumbnail.
pub fn generate_thumbnail(
    _song_name: &str,
    _artist: &str,
    output_path: &Path,
) -> Result<PathBuf> {
    // YouTube-recommended thumbnail size
    let width: u32 = 1280;
    let height: u32 = 720;

    let mut img = RgbaImage::new(width, height);

    // Draw gradient background (dark purple to deep blue)
    for y in 0..height {
        for x in 0..width {
            let t_x = x as f64 / width as f64;
            let t_y = y as f64 / height as f64;
            let t = (t_x + t_y) / 2.0;

            // Gradient from #0f0c29 to #302b63 to #24243e
            let r = lerp(15.0, 48.0, t) as u8;
            let g = lerp(12.0, 36.0, t) as u8;
            let b = lerp(41.0, 62.0, t) as u8;

            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // Draw a subtle music note icon in the center (large, semi-transparent)
    draw_music_note(&mut img, width / 2, height / 2 - 40, 120, 60);

    // We can't easily render text without a font file, so we'll skip text
    // and rely on the gradient + music note design. The user can add a
    // font file later if they want text on thumbnails.

    let thumb_path = output_path.join("thumbnail.png");
    img.save_with_format(&thumb_path, ImageFormat::Png)?;
    Ok(thumb_path)
}

/// Linear interpolation helper.
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Draws a simple music note silhouette at the given position.
fn draw_music_note(img: &mut RgbaImage, cx: u32, cy: u32, scale_x: u32, scale_y: u32) {
    // Simple two-note symbol drawn as ellipses and a stem
    let alpha: u8 = 80; // Semi-transparent white

    // First note head (ellipse)
    let head1_cx = cx - scale_x / 3;
    let head1_cy = cy + scale_y / 2;
    let rx1 = scale_x / 5;
    let ry1 = scale_y / 5;
    draw_ellipse(img, head1_cx, head1_cy, rx1, ry1, Rgba([255, 255, 255, alpha]));

    // Second note head (ellipse)
    let head2_cx = cx + scale_x / 4;
    let head2_cy = cy + scale_y / 3;
    let rx2 = scale_x / 5;
    let ry2 = scale_y / 5;
    draw_ellipse(img, head2_cx, head2_cy, rx2, ry2, Rgba([255, 255, 255, alpha]));

    // Stem for first note
    let stem1_x = head1_cx + rx1 - 3;
    for y in cy - scale_y / 2..=head1_cy {
        if y < img.height() {
            for dx in 0..6 {
                let px = stem1_x as i64 + dx as i64;
                let py = y as i64;
                if px >= 0 && px < img.width() as i64 && py >= 0 && py < img.height() as i64 {
                    img.put_pixel(px as u32, py as u32, Rgba([255, 255, 255, alpha]));
                }
            }
        }
    }

    // Stem for second note
    let stem2_x = head2_cx + rx2 - 3;
    for y in cy - scale_y / 3..=head2_cy {
        if y < img.height() {
            for dx in 0..6 {
                let px = stem2_x as i64 + dx as i64;
                let py = y as i64;
                if px >= 0 && px < img.width() as i64 && py >= 0 && py < img.height() as i64 {
                    img.put_pixel(px as u32, py as u32, Rgba([255, 255, 255, alpha]));
                }
            }
        }
    }

    // Beam connecting the two stems
    let beam_y = cy - scale_y / 2;
    if beam_y < img.height() {
        for x in stem1_x..=stem2_x + 6 {
            for dy in 0..8 {
                let px = x as i64;
                let py = beam_y as i64 + dy as i64;
                if px >= 0 && px < img.width() as i64 && py >= 0 && py < img.height() as i64 {
                    img.put_pixel(px as u32, py as u32, Rgba([255, 255, 255, alpha]));
                }
            }
        }
    }
}

/// Draws a filled ellipse using midpoint algorithm.
fn draw_ellipse(
    img: &mut RgbaImage,
    cx: u32,
    cy: u32,
    rx: u32,
    ry: u32,
    color: Rgba<u8>,
) {
    if rx == 0 || ry == 0 {
        return;
    }

    let (w, h) = img.dimensions();

    for y in 0..h {
        let dy = y as f64 - cy as f64;
        if dy.abs() > ry as f64 {
            continue;
        }
        let max_dx = rx as f64 * (1.0 - (dy / ry as f64).powi(2)).sqrt();
        for x in (cx as i64 - max_dx as i64)..=(cx as i64 + max_dx as i64) {
            if x >= 0 && x < w as i64 {
                img.put_pixel(x as u32, y, color);
            }
        }
    }
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

