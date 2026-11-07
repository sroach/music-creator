use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Find an FFmpeg binary that supports the `drawtext` filter.
///
/// Tries `ffmpeg-full` (Homebrew) first, then falls back to the default `ffmpeg` in PATH.
fn find_ffmpeg() -> String {
    // Candidates in order of preference
    let candidates = [
        "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg", // Homebrew ffmpeg-full on Apple Silicon
        "/usr/local/opt/ffmpeg-full/bin/ffmpeg",    // Homebrew ffmpeg-full on Intel
        "ffmpeg",                                    // Default ffmpeg in PATH
    ];

    for candidate in &candidates {
        if let Ok(output) = Command::new(candidate)
            .args(["-filters"])
            .output()
        {
            let filters = String::from_utf8_lossy(&output.stdout);
            if filters.contains("drawtext") {
                return candidate.to_string();
            }
        }
    }

    // Fallback to default ffmpeg even without drawtext
    "ffmpeg".to_string()
}

/// Check if the given FFmpeg binary supports the `drawtext` filter.
fn has_drawtext(ffmpeg_path: &str) -> bool {
    if let Ok(output) = Command::new(ffmpeg_path).args(["-filters"]).output() {
        let filters = String::from_utf8_lossy(&output.stdout);
        return filters.contains("drawtext");
    }
    false
}

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
/// Uses the showfreqs filter to generate an animated frequency visualization (equalizer bars)
/// from the MP3 audio, with an optional thumbnail overlay and optional song title text.
/// Creates an H.264/AAC MP4 suitable for YouTube upload.
pub fn convert_mp3_to_mp4(
    mp3_path: &Path,
    output_dir: &Path,
    title: Option<&str>,
) -> Result<String> {
    // Find FFmpeg binary with drawtext support
    let ffmpeg_path = find_ffmpeg();
    let ffmpeg_has_drawtext = has_drawtext(&ffmpeg_path);

    // Check if ffmpeg is available
    let ffmpeg_check = Command::new(&ffmpeg_path).arg("-version").output();
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

    // Derive output filename from input
    let mp4_filename = mp3_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string()
        + ".mp4";

    let mp4_path = output_dir.join(&mp4_filename);

    // Check if a thumbnail exists for this song
    let safe_name = mp3_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let thumbnail_path = output_dir.join(format!("{}.png", safe_name));
    let has_thumbnail = thumbnail_path.exists();

    // Build the filter complex based on whether a thumbnail exists
    // Frequency analyzer settings: bar mode for distinct vertical equalizer bars,
    // 1200x180 size, gold-to-pink gradient (0xFFCC00|0xE60067), centered horizontally and 10px from the bottom edge.
    // Semi-transparent dark box behind bars: 200px tall, full width, dark plum at 60% opacity,
    // anchored to bottom - bars sit inside with 10px padding top/bottom.
    let freq_opts = "s=1200x180:mode=bar:colors=0xFFCC00|0xE60067:rate=30:ascale=sqrt";
    let box_opts = "y=ih-200:h=200:w=iw:color=0x1A0A16@0.6:t=fill";
    let freq_overlay = "(W-w)/2:H-h-10";

    // Song title text overlay: top center, white bold text with semi-transparent dark background box
    // matching the equalizer box style for visual consistency.
    // Only use drawtext if the FFmpeg binary supports it.
    let title_filter = if ffmpeg_has_drawtext {
        title.and_then(|t| {
            // Escape single quotes for FFmpeg drawtext (replace ' with '\'')
            let escaped = t.replace('\'', "'\\''");
            Some(format!(
                ",drawtext=text='{}':fontcolor=white:fontsize=48:fontfile=/System/Library/Fonts/Supplemental/Arial\\ Bold.ttf:x=(W-w)/2:y=h*0.05:box=1:boxcolor=0x1A0A16@0.6:boxborderw=25",
                escaped
            ))
        }).unwrap_or_default()
    } else {
        if title.is_some() {
            eprintln!("Warning: FFmpeg does not support 'drawtext' filter — song title will not be displayed. Install ffmpeg-full via 'brew install ffmpeg-full'.");
        }
        String::new()
    };

    let filter = if has_thumbnail {
        // Full-size thumbnail as background, drawbox behind bars, then frequency bars overlay, optional title
        format!(
            "[1:v]scale=1920:1080:force_original_aspect_ratio=decrease,setsar=1[thumb];[0:a]showfreqs={}[wave];[thumb]drawbox={}[bg_with_box];[bg_with_box][wave]overlay={}{}[outv]",
            freq_opts, box_opts, freq_overlay, title_filter
        )
    } else {
        // Dark background, drawbox behind bars, then frequency bars overlay, optional title
        format!(
            "color=s=1920x1080:r=30:c=0x000000[bg];[0:a]showfreqs={}[wave];[bg]drawbox={}[bg_with_box];[bg_with_box][wave]overlay={}{}[outv]",
            freq_opts, box_opts, freq_overlay, title_filter
        )
    };

    // Run FFmpeg to convert MP3 to MP4 with animated waveform visualization
    let status = if has_thumbnail {
        let thumb_str = thumbnail_path.to_str().unwrap();
        Command::new(&ffmpeg_path)
            .args([
                "-y", // Overwrite output file
                "-i", mp3_path.to_str().unwrap(), // Input audio (stream 0)
                "-loop", "1", // Loop the image
                "-i", thumb_str, // Input thumbnail image (stream 1)
                "-c:v", "libx264", // H.264 video codec
                "-c:a", "aac", // AAC audio codec
                "-pix_fmt", "yuv420p", // Required for YouTube compatibility
                "-filter_complex", &filter, // Complex filter for waveform + thumbnail
                "-map", "[outv]", // Map the final video output from filter
                "-map", "0:a", // Map audio from input 0 (the MP3)
                "-shortest", // Stop when audio ends
                "-movflags", "+faststart", // Optimize for web streaming
                mp4_path.to_str().unwrap(),
            ])
            .output()?
    } else {
        Command::new(&ffmpeg_path)
            .args([
                "-y", // Overwrite output file
                "-i", mp3_path.to_str().unwrap(), // Input audio
                "-c:v", "libx264", // H.264 video codec
                "-c:a", "aac", // AAC audio codec
                "-pix_fmt", "yuv420p", // Required for YouTube compatibility
                "-filter_complex", &filter, // Complex filter for waveform on dark background
                "-map", "[outv]", // Map the final video output from filter
                "-map", "0:a", // Map audio from input 0 (the MP3)
                "-shortest", // Stop when audio ends
                "-movflags", "+faststart", // Optimize for web streaming
                mp4_path.to_str().unwrap(),
            ])
            .output()?
    };

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("FFmpeg failed: {}", stderr);
    }

    println!("🎬 Converted {} to {}", mp3_path.display(), mp4_path.display());
    Ok(mp4_filename)
}


