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
| `passkey-auth` | WebAuthn passkey authentication |
| `cookie` | Session cookie management |
| `uuid` | Session ID generation |

## Admin Panel

The app includes a secure admin panel behind `/admin/` for uploading, editing, and managing songs with WebAuthn passkey authentication (Touch ID, Face ID, hardware keys).

### Admin Features

- **Passkey Authentication** — Register your device once, then sign in with Touch ID/Face ID
- **Song Upload** — Upload MP3 + JSON metadata + optional PNG/MP4 files
- **Song Edit** — Replace individual files (MP3, PNG, JSON) for existing songs
- **Song Delete** — Remove songs from output/ or accepted/ directories
- **Song Move** — Move songs between output/ and accepted/ directories

### Admin Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/admin/login` | Admin login page |
| `GET` | `/admin/register` | Passkey registration page |
| `GET` | `/admin/dashboard` | Song management dashboard |
| `GET` | `/admin/upload` | Upload new song form |
| `GET` | `/admin/edit/{filename}` | Edit existing song |
| `POST` | `/admin/register/start` | Start passkey registration |
| `POST` | `/admin/register/finish` | Complete passkey registration |
| `POST` | `/admin/login/start` | Start passkey authentication |
| `POST` | `/admin/login/finish` | Complete passkey authentication |
| `GET` | `/admin/logout` | Destroy session |
| `GET` | `/admin/songs` | List all songs (JSON) |
| `POST` | `/admin/upload` | Upload new song (multipart) |
| `PUT` | `/admin/song/{filename}` | Update song files (multipart) |
| `DELETE` | `/admin/song/{filename}` | Delete song |
| `POST` | `/admin/song/{filename}/move` | Move song between directories |

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ADMIN_RP_ID` | WebAuthn RP ID (domain) for passkey auth | `localhost` |

### Nginx Configuration

Example nginx configuration for proxying the admin panel with HTTPS:

```nginx
server {
    listen 443 ssl http2;
    server_name music.example.com;

    ssl_certificate     /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # WebAuthn requires HTTPS (or localhost)
    location /admin/ {
        proxy_pass http://127.0.0.1:8001/admin/;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        client_max_body_size 100M;  # For large MP3/MP4 uploads
    }

    location / {
        proxy_pass http://127.0.0.1:8001/;
        proxy_set_header Host $host;
    }
}
```

Set `ADMIN_RP_ID=music.example.com` when running behind nginx so passkeys work correctly.
