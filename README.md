# Music Creator

A web-based music generation tool that uses the [ACEMusic AI](https://acemusic.ai) API to generate songs, with on-device lyrics generation and AI-powered album art thumbnails.

## Features

- **Song Generation** — Generate MP3 songs via the ACEMusic AI API with custom name, artist, lyrics, style prompt, duration, and BPM.
- **Lyrics Generation** — On-device lyrics generation using Apple Foundation Models (macOS 26+).
- **AI Album Art** — Generate album art thumbnails using Stable Diffusion (Anima model) via `diffusion-rs`.
- **Video Conversion** — Convert MP3 songs to MP4 videos with album art thumbnails.
- **Web UI** — Browser-based interface for generating, playing, and managing songs.

## Requirements

- **Rust** 1.75+ (edition 2024)
- **macOS 26+** — Required for Apple Foundation Models (lyrics generation)
- **Apple Silicon** — Recommended for faster AI thumbnail generation via Metal GPU

## Installation

```bash
git clone <repository-url>
cd music-creator
cargo build --release
```

## Running

### Quick Start

```bash
# Set your ACEMusic API key
export ACESTEP_API_KEY="your-api-key-here"

# Run the server
cargo run --release
```

The server starts at `http://localhost:8001`.

### CLI Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `--api-key <KEY>` | ACEMusic API key (overrides `ACESTEP_API_KEY` env var) | — |
| `--port <PORT>` | Server port (overrides `PORT` env var) | `8001` |
| `--output-dir <DIR>` | Output directory for generated files | `output` |

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ACESTEP_API_KEY` | ACEMusic API key | — |
| `IMAGE_GEN_STEPS` | Diffusion steps for AI thumbnails (higher = better quality, slower) | `30` |
| `IMAGE_GEN_SEED` | Random seed for reproducible thumbnails | `299792458` |

### Configuration Priority

1. **CLI arguments** — Highest priority
2. **Environment variables** — Second priority
3. **`.env` file** — Lowest priority (loaded automatically if present)

Example `.env` file:
```
ACESTEP_API_KEY=your-api-key-here
IMAGE_GEN_STEPS=30
IMAGE_GEN_SEED=299792458
```

## AI Thumbnail Generation

Thumbnails are generated using the **Anima** Stable Diffusion model (~7GB, downloaded on first use). The model is cached in the Hugging Face hub directory after the first download.

- First thumbnail generation may take several minutes while the model downloads.
- Subsequent generations are faster, especially on Apple Silicon with Metal GPU acceleration.
- A 120-second timeout applies; if AI generation fails or times out, a procedural gradient thumbnail is used as a fallback.

### Thumbnail Preset

The current preset is `Anima` (Q8_0 quantization) for anime-style artistic album art. To change the preset, edit `src/image_gen.rs`.

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/` | Web UI |
| `POST` | `/generate` | Generate a new song (SSE stream) |
| `POST` | `/generate-lyrics` | Generate lyrics using Foundation Models |
| `GET` | `/list` | List all generated songs |
| `GET` | `/song/{filename}` | Get song details |
| `GET` | `/download/{filename}` | Download a file (MP3, MP4, or PNG) |
| `POST` | `/thumbnail/{filename}` | Generate AI thumbnail for a song |
| `POST` | `/convert/{filename}` | Convert MP3 to MP4 with thumbnail |

## Project Structure

```
├── src/
│   ├── main.rs         — Axum web server, API endpoints
│   ├── convert.rs      — MP3 to MP4 conversion, procedural thumbnails
│   ├── image_gen.rs    — AI thumbnail generation via diffusion-rs
│   └── lyrics_gen.rs   — On-device lyrics generation via Foundation Models
├── static/
│   └── index.html      — Web UI
├── output/             — Generated MP3, MP4, and PNG files
└── Cargo.toml          — Dependencies
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `axum` | Web framework |
| `diffusion-rs` | Stable Diffusion image generation |
| `foundation-models` | Apple on-device AI (lyrics) |
| `reqwest` | HTTP client for ACEMusic API |
| `image` | Procedural thumbnail fallback |
| `id3` | MP3 metadata (ID3 tags) |
| `clap` | CLI argument parsing |
| `dotenvy` | `.env` file support |
