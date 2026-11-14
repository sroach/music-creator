//! Admin module: WebAuthn passkey authentication, sessions, and song management.

use crate::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Json,
};
use chrono::Utc;
use cookie::{Cookie, Key};
use passkey_auth::{
    AuthenticationResponse, AuthenticationState, CredentialId, PasskeyCredential,
    RegistrationResponse, RegistrationState, Webauthn,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// Embedded admin HTML pages
static ADMIN_LOGIN_HTML: &str = include_str!("../static/admin-login.html");
static ADMIN_REGISTER_HTML: &str = include_str!("../static/admin-register.html");
static ADMIN_DASHBOARD_HTML: &str = include_str!("../static/admin-dashboard.html");
static ADMIN_UPLOAD_HTML: &str = include_str!("../static/admin-upload.html");
static ADMIN_EDIT_HTML: &str = include_str!("../static/admin-edit.html");

const SESSION_COOKIE_NAME: &str = "admin_session";
const SESSION_MAX_AGE_SECS: u64 = 86400;

/// In-memory challenge store (between start and finish of ceremony)
struct ChallengeStore {
    registrations: HashMap<Uuid, RegistrationState>,
    authentications: HashMap<Uuid, AuthenticationState>,
}

/// Admin state: passkey credentials, sessions, and WebAuthn server
pub struct AdminState {
    webauthn: Webauthn,
    _cookie_key: Key,
    secure_cookies: bool,
    credentials: Mutex<Vec<StoredCredential>>,
    sessions: Mutex<HashMap<Uuid, SessionEntry>>,
    challenges: Mutex<ChallengeStore>,
    admin_json_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredential {
    credential_id: String,
    public_key: String,
    counter: u32,
    transports: Vec<String>,
    aaguid: Vec<u8>,
    user_handle: String,
    display_name: String,
    created_at: String,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    user_handle: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AdminStorage {
    credentials: Vec<StoredCredential>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterStartRequest {
    #[serde(default)]
    display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterFinishRequest {
    challenge_id: String,
    #[serde(flatten)]
    response: RegistrationResponse,
}

#[derive(Debug, Deserialize)]
pub struct LoginStartRequest {
    #[serde(default)]
    user_handle: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginFinishRequest {
    challenge_id: String,
    #[serde(flatten)]
    response: AuthenticationResponse,
}

#[derive(Debug, Deserialize)]
pub struct MoveSongRequest {
    target: String,
}

#[derive(Debug, Deserialize)]
pub struct AdminPageQuery {
    #[serde(default)]
    error: String,
}

impl AdminState {
    pub fn new(rp_id: String, port: u16, admin_json_path: PathBuf) -> Self {
        // For localhost development, use http://localhost:{port} as the origin.
        // For production domains, use https://{domain}.
        let is_localhost = rp_id == "localhost";
        let origin = if is_localhost {
            format!("http://localhost:{}", port)
        } else {
            format!("https://{}", rp_id)
        };
        let webauthn = Webauthn::new(&rp_id, "Music Creator Admin", &origin)
            .require_user_verification(true);
        let cookie_key = Key::generate();
        let credentials = load_credentials(&admin_json_path);

        Self {
            webauthn,
            _cookie_key: cookie_key,
            secure_cookies: !is_localhost,
            credentials: Mutex::new(credentials),
            sessions: Mutex::new(HashMap::new()),
            challenges: Mutex::new(ChallengeStore {
                registrations: HashMap::new(),
                authentications: HashMap::new(),
            }),
            admin_json_path,
        }
    }

    pub async fn validate_session(&self, cookie_header: Option<&str>) -> Option<String> {
        let sessions = self.sessions.lock().await;
        if let Some(header) = cookie_header {
            for part in header.split(';') {
                let part = part.trim();
                if let Ok(cookie) = Cookie::parse(part) {
                    if cookie.name() == SESSION_COOKIE_NAME {
                        if let Ok(session_id) = Uuid::parse_str(cookie.value()) {
                            if let Some(session) = sessions.get(&session_id) {
                                if Utc::now() < session.expires_at {
                                    return Some(session.user_handle.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    async fn create_session(&self, user_handle: &str) -> Uuid {
        let session_id = Uuid::new_v4();
        let session = SessionEntry {
            user_handle: user_handle.to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(SESSION_MAX_AGE_SECS as i64),
        };
        self.sessions.lock().await.insert(session_id, session);
        session_id
    }

    async fn destroy_session(&self, session_id: &Uuid) {
        self.sessions.lock().await.remove(session_id);
    }

    async fn save_credentials(&self) {
        let creds = self.credentials.lock().await.clone();
        let storage = AdminStorage { credentials: creds };
        if let Ok(json) = serde_json::to_string_pretty(&storage) {
            let _ = fs::write(&self.admin_json_path, json);
        }
    }

    fn stored_to_passkey(sc: &StoredCredential) -> Option<PasskeyCredential> {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let id = URL_SAFE_NO_PAD.decode(&sc.credential_id).ok()?;
        let pk = URL_SAFE_NO_PAD.decode(&sc.public_key).ok()?;
        let mut aaguid = [0u8; 16];
        if sc.aaguid.len() == 16 {
            aaguid.copy_from_slice(&sc.aaguid);
        }
        Some(PasskeyCredential {
            id: CredentialId(id),
            public_key_cose: passkey_auth::CosePublicKey(pk),
            counter: sc.counter,
            transports: sc.transports.clone(),
            aaguid,
        })
    }

    fn passkey_to_stored(
        pc: &PasskeyCredential,
        user_handle: &str,
        display_name: &str,
    ) -> StoredCredential {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        StoredCredential {
            credential_id: URL_SAFE_NO_PAD.encode(pc.id.as_bytes()),
            public_key: URL_SAFE_NO_PAD.encode(pc.public_key_cose.as_bytes()),
            counter: pc.counter,
            transports: pc.transports.clone(),
            aaguid: pc.aaguid.to_vec(),
            user_handle: user_handle.to_string(),
            display_name: display_name.to_string(),
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

fn load_credentials(path: &PathBuf) -> Vec<StoredCredential> {
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(storage) = serde_json::from_str::<AdminStorage>(&content) {
            return storage.credentials;
        }
    }
    Vec::new()
}

fn build_session_cookie(session_id: &Uuid, secure: bool) -> String {
    let secure_flag = if secure { "Secure;" } else { "" };
    format!(
        "{}={}; HttpOnly; Path=/; {}SameSite=Lax; MaxAge={}",
        SESSION_COOKIE_NAME,
        session_id,
        secure_flag,
        SESSION_MAX_AGE_SECS
    )
}

fn build_clear_cookie(secure: bool) -> String {
    let secure_flag = if secure { "Secure;" } else { "" };
    format!(
        "{}=; HttpOnly; Path=/; {}SameSite=Lax; MaxAge=0",
        SESSION_COOKIE_NAME,
        secure_flag
    )
}

async fn check_auth(state: &Arc<AppState>, headers: &HeaderMap) -> Result<String, StatusCode> {
    let cookie_header = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Some(user) = state.admin.validate_session(cookie_header).await {
        Ok(user)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

// ─── Page Handlers ─────────────────────────────────────────────────────

pub async fn admin_login_page(Query(query): Query<AdminPageQuery>) -> impl IntoResponse {
    let html = if !query.error.is_empty() {
        ADMIN_LOGIN_HTML.replace("<!-- ERROR -->", &format!("<p class='error-message'>{}</p>", query.error))
    } else {
        ADMIN_LOGIN_HTML.to_string()
    };
    Html(html).into_response()
}

pub async fn admin_register_page() -> impl IntoResponse {
    Html(ADMIN_REGISTER_HTML.to_string()).into_response()
}

pub async fn admin_dashboard_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        use axum::http::HeaderValue;
        let location = HeaderValue::from_static("/admin/login");
        return (
            StatusCode::SEE_OTHER,
            [(axum::http::header::LOCATION, location)],
            Body::empty(),
        )
            .into_response();
    }
    Html(ADMIN_DASHBOARD_HTML.to_string()).into_response()
}

pub async fn admin_upload_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        use axum::http::HeaderValue;
        let location = HeaderValue::from_static("/admin/login");
        return (
            StatusCode::SEE_OTHER,
            [(axum::http::header::LOCATION, location)],
            Body::empty(),
        )
            .into_response();
    }
    Html(ADMIN_UPLOAD_HTML.to_string()).into_response()
}

pub async fn admin_edit_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        use axum::http::HeaderValue;
        let location = HeaderValue::from_static("/admin/login");
        return (
            StatusCode::SEE_OTHER,
            [(axum::http::header::LOCATION, location)],
            Body::empty(),
        )
            .into_response();
    }
    let filename = filename.trim_end_matches(".mp3");
    let html = ADMIN_EDIT_HTML.replace("{{FILENAME}}", filename);
    Html(html).into_response()
}

// ─── Passkey Registration ──────────────────────────────────────────────

pub async fn register_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterStartRequest>,
) -> impl IntoResponse {
    let display_name = if req.display_name.is_empty() {
        "Admin".to_string()
    } else {
        req.display_name
    };
    let user_handle = "admin-1";

    let creds = state.admin.credentials.lock().await;
    let existing: Vec<CredentialId> = creds
        .iter()
        .filter_map(|sc| {
            use base64::Engine;
            use base64::engine::general_purpose::URL_SAFE_NO_PAD;
            URL_SAFE_NO_PAD.decode(&sc.credential_id).ok().map(CredentialId)
        })
        .collect();
    drop(creds);

    let (challenge, reg_state) = state.admin.webauthn.start_registration(
        user_handle.as_bytes(),
        "admin@music-creator",
        &display_name,
        &existing,
    );

    let challenge_id = Uuid::new_v4();
    state.admin.challenges.lock().await.registrations.insert(challenge_id, reg_state);

    let json_challenge = serde_json::to_value(&challenge).unwrap_or_else(|_| {
        serde_json::json!({
            "challenge": "",
            "rp": {"id": "", "name": ""},
            "user": {"id": "", "name": "", "displayName": ""},
            "pubKeyCredParams": [],
            "timeout": 60000,
            "authenticatorSelection": {},
            "attestation": "none"
        })
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challenge_id": challenge_id.to_string(),
            "options": json_challenge
        })),
    )
        .into_response()
}

pub async fn register_finish(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterFinishRequest>,
) -> impl IntoResponse {
    let challenge_id = match Uuid::parse_str(&req.challenge_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid challenge ID"})),
            )
                .into_response();
        }
    };

    let reg_state = match state
        .admin
        .challenges
        .lock()
        .await
        .registrations
        .remove(&challenge_id)
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Challenge expired or invalid"})),
            )
                .into_response();
        }
    };

    match state.admin.webauthn.finish_registration(&reg_state, &req.response) {
        Ok(credential) => {
            let user_handle = "admin-1";
            let stored = AdminState::passkey_to_stored(&credential, user_handle, "Admin");

            {
                let mut creds = state.admin.credentials.lock().await;
                creds.push(stored);
            }
            state.admin.save_credentials().await;

            let session_id = state.admin.create_session(user_handle).await;
            let set_cookie = build_session_cookie(&session_id, state.admin.secure_cookies);

            use axum::http::HeaderValue;
            let set_cookie_val = HeaderValue::from_str(&set_cookie).unwrap();
            let content_type = HeaderValue::from_static("application/json");
            (
                StatusCode::OK,
                [(axum::http::header::SET_COOKIE, set_cookie_val), (axum::http::header::CONTENT_TYPE, content_type)],
                Json(serde_json::json!({
                    "message": "Registration successful",
                    "redirect": "/admin/dashboard"
                })),
            )
                .into_response()
        }
        Err(e) => {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Registration failed: {}", e)})),
            )
                .into_response()
        }
    }
}

// ─── Passkey Authentication ────────────────────────────────────────────

pub async fn login_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginStartRequest>,
) -> impl IntoResponse {
    let user_handle = if req.user_handle.is_empty() {
        "admin-1".to_string()
    } else {
        req.user_handle
    };

    let creds = state.admin.credentials.lock().await;
    let user_creds: Vec<PasskeyCredential> = creds
        .iter()
        .filter(|c| c.user_handle == user_handle)
        .filter_map(|sc| AdminState::stored_to_passkey(sc))
        .collect();
    drop(creds);

    if user_creds.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No registered passkeys found"})),
        )
            .into_response();
    }

    let (challenge, auth_state) = state
        .admin
        .webauthn
        .start_authentication_with_creds_for_user(user_handle.as_bytes(), &user_creds);

    let challenge_id = Uuid::new_v4();
    state
        .admin
        .challenges
        .lock()
        .await
        .authentications
        .insert(challenge_id, auth_state);

    let json_challenge = serde_json::to_value(&challenge).unwrap_or_else(|_| {
        serde_json::json!({
            "challenge": "",
            "rpId": "",
            "timeout": 60000,
            "allowCredentials": [],
            "userVerification": "preferred"
        })
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challenge_id": challenge_id.to_string(),
            "options": json_challenge
        })),
    )
        .into_response()
}

pub async fn login_finish(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginFinishRequest>,
) -> impl IntoResponse {
    let challenge_id = match Uuid::parse_str(&req.challenge_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid challenge ID"})),
            )
                .into_response();
        }
    };

    let auth_state = match state
        .admin
        .challenges
        .lock()
        .await
        .authentications
        .remove(&challenge_id)
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Challenge expired or invalid"})),
            )
                .into_response();
        }
    };

    // Find the stored credential by ID
    let asserted_id_b64 = req.response.id.clone();
    let creds = state.admin.credentials.lock().await;
    let stored_cred = creds
        .iter()
        .find(|sc| sc.credential_id == asserted_id_b64)
        .and_then(|sc| AdminState::stored_to_passkey(sc));
    drop(creds);

    let stored = match stored_cred {
        Some(c) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Credential not found"})),
            )
                .into_response();
        }
    };

    match state
        .admin
        .webauthn
        .finish_authentication(&auth_state, &req.response, &stored)
    {
        Ok(success) => {
            // Update counter
            {
                let mut creds = state.admin.credentials.lock().await;
                for cred in creds.iter_mut() {
                    if cred.credential_id == asserted_id_b64 {
                        cred.counter = success.new_counter;
                        break;
                    }
                }
            }
            state.admin.save_credentials().await;

            // Extract user handle from auth state
            let user_handle = auth_state
                .user_handle
                .as_deref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("admin-1")
                .to_string();

            let session_id = state.admin.create_session(&user_handle).await;
            let set_cookie = build_session_cookie(&session_id, state.admin.secure_cookies);

            use axum::http::HeaderValue;
            let set_cookie_val = HeaderValue::from_str(&set_cookie).unwrap();
            let content_type = HeaderValue::from_static("application/json");
            (
                StatusCode::OK,
                [(axum::http::header::SET_COOKIE, set_cookie_val), (axum::http::header::CONTENT_TYPE, content_type)],
                Json(serde_json::json!({
                    "message": "Authentication successful",
                    "redirect": "/admin/dashboard"
                })),
            )
                .into_response()
        }
        Err(e) => {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": format!("Authentication failed: {}", e)})),
            )
                .into_response()
        }
    }
}

// ─── Logout ────────────────────────────────────────────────────────────

pub async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let cookie_header = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Some(header) = cookie_header {
        for part in header.split(';') {
            let part = part.trim();
            if let Ok(cookie) = Cookie::parse(part) {
                if cookie.name() == SESSION_COOKIE_NAME {
                    if let Ok(session_id) = Uuid::parse_str(cookie.value()) {
                        state.admin.destroy_session(&session_id).await;
                    }
                }
            }
        }
    }

    let clear_cookie = build_clear_cookie(state.admin.secure_cookies);
    use axum::http::HeaderValue;
    let clear_cookie_val = HeaderValue::from_str(&clear_cookie).unwrap();
    let location = HeaderValue::from_static("/admin/login");
    (
        StatusCode::SEE_OTHER,
        [(axum::http::header::SET_COOKIE, clear_cookie_val), (axum::http::header::LOCATION, location)],
        Body::empty(),
    )
        .into_response()
}

// ─── Song List ─────────────────────────────────────────────────────────

pub async fn list_songs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    use crate::scan_songs_in_dir;

    let output_dir = PathBuf::from(&state.output_dir);
    let accepted_dir = PathBuf::from(&state.accepted_dir);

    let mut songs = Vec::new();
    songs.extend(scan_songs_in_dir(&output_dir, false));
    songs.extend(scan_songs_in_dir(&accepted_dir, true));

    songs.sort_by(|a, b| {
        b.accepted
            .cmp(&a.accepted)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });

    (StatusCode::OK, Json(songs)).into_response()
}

// ─── Song Upload ───────────────────────────────────────────────────────

pub async fn upload_song(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let mut mp3_data: Option<Vec<u8>> = None;
    let mut json_data: Option<String> = None;
    let mut png_data: Option<Vec<u8>> = None;
    let mut mp4_data: Option<Vec<u8>> = None;
    let mut song_name = String::new();
    let mut artist = String::new();
    let mut prompt = String::new();
    let mut lyrics = String::new();
    let mut duration: u32 = 180;
    let mut bpm: u32 = 120;

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().map(|s| s.to_string());
        let data = field.bytes().await.unwrap();

        match name.as_deref() {
            Some("mp3") => mp3_data = Some(data.to_vec()),
            Some("json") => json_data = Some(String::from_utf8_lossy(&data).to_string()),
            Some("png") => png_data = Some(data.to_vec()),
            Some("mp4") => mp4_data = Some(data.to_vec()),
            Some("song_name") => song_name = String::from_utf8_lossy(&data).to_string(),
            Some("artist") => artist = String::from_utf8_lossy(&data).to_string(),
            Some("prompt") => prompt = String::from_utf8_lossy(&data).to_string(),
            Some("lyrics") => lyrics = String::from_utf8_lossy(&data).to_string(),
            Some("duration") => {
                if let Ok(d) = String::from_utf8_lossy(&data).trim().parse() {
                    duration = d;
                }
            }
            Some("bpm") => {
                if let Ok(b) = String::from_utf8_lossy(&data).trim().parse() {
                    bpm = b;
                }
            }
            _ => {}
        }
    }

    if mp3_data.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "MP3 file is required"})),
        )
            .into_response();
    }

    let safe_name = song_name
        .replace(' ', "_")
        .replace(|c: char| !c.is_alphanumeric() && c != '_', "");
    let filename = if safe_name.is_empty() {
        format!("song_{}.mp3", Utc::now().timestamp())
    } else {
        format!("{}.mp3", safe_name)
    };

    let base = filename.trim_end_matches(".mp3");
    let accepted_dir = PathBuf::from(&state.accepted_dir);
    fs::create_dir_all(&accepted_dir).unwrap_or_else(|_| ());

    let mp3_path = accepted_dir.join(&filename);
    if let Some(data) = mp3_data {
        if let Err(e) = fs::write(&mp3_path, &data) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to save MP3: {}", e)})),
            )
                .into_response();
        }
    }

    if let Some(data) = png_data {
        let _ = fs::write(accepted_dir.join(format!("{}.png", base)), &data);
    }

    if let Some(data) = mp4_data {
        let _ = fs::write(accepted_dir.join(format!("{}.mp4", base)), &data);
    }

    let file_size = mp3_path.metadata().map(|m| m.len() as usize).unwrap_or(0);

    if let Some(json_str) = &json_data {
        let _ = fs::write(accepted_dir.join(format!("{}.json", base)), json_str);
    } else {
        let metadata = crate::SongMetadata {
            song_name: if song_name.is_empty() { "Unknown".to_string() } else { song_name },
            artist: if artist.is_empty() { "Unknown".to_string() } else { artist },
            prompt: if prompt.is_empty() { "Uploaded".to_string() } else { prompt },
            lyrics,
            duration,
            bpm,
            model: "upload".to_string(),
            filename: filename.clone(),
            created_at: Utc::now().to_rfc3339(),
            file_size_bytes: file_size,
        };
        if let Ok(json_str) = serde_json::to_string_pretty(&metadata) {
            let _ = fs::write(accepted_dir.join(format!("{}.json", base)), json_str);
        }
    }

    crate::sync_m3u_playlist(&accepted_dir);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": format!("Song uploaded: {}", filename),
            "filename": filename,
        })),
    )
        .into_response()
}

// ─── Song Update ───────────────────────────────────────────────────────

pub async fn update_song(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let filename = if filename.ends_with(".mp3") {
        filename
    } else {
        format!("{}.mp3", filename)
    };
    let base = filename.trim_end_matches(".mp3");

    let output_dir = PathBuf::from(&state.output_dir);
    let accepted_dir = PathBuf::from(&state.accepted_dir);

    let song_dir = if output_dir.join(&filename).exists() {
        output_dir
    } else if accepted_dir.join(&filename).exists() {
        accepted_dir
    } else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Song not found"})),
        )
            .into_response();
    };

    let mut has_updates = false;

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().map(|s| s.to_string());
        let data = field.bytes().await.unwrap();

        match name.as_deref() {
            Some("mp3") => {
                if fs::write(song_dir.join(&filename), &data).is_ok() {
                    has_updates = true;
                }
            }
            Some("png") => {
                if fs::write(song_dir.join(format!("{}.png", base)), &data).is_ok() {
                    has_updates = true;
                }
            }
            Some("json") => {
                if fs::write(song_dir.join(format!("{}.json", base)), &data).is_ok() {
                    has_updates = true;
                }
            }
            Some("mp4") => {
                if fs::write(song_dir.join(format!("{}.mp4", base)), &data).is_ok() {
                    has_updates = true;
                }
            }
            _ => {}
        }
    }

    if !has_updates {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No files to update"})),
        )
            .into_response();
    }

    crate::sync_m3u_playlist(&song_dir);

    (
        StatusCode::OK,
        Json(serde_json::json!({"message": format!("Song updated: {}", filename)})),
    )
        .into_response()
}

// ─── Song Delete ───────────────────────────────────────────────────────

pub async fn delete_song(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let filename = if filename.ends_with(".mp3") {
        filename
    } else {
        format!("{}.mp3", filename)
    };
    let base = filename.trim_end_matches(".mp3");

    let output_dir = PathBuf::from(&state.output_dir);
    let accepted_dir = PathBuf::from(&state.accepted_dir);

    let song_dir = if output_dir.join(&filename).exists() {
        output_dir
    } else if accepted_dir.join(&filename).exists() {
        accepted_dir
    } else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Song not found"})),
        )
            .into_response();
    };

    let mut deleted = Vec::new();
    for (ext, label) in [(".mp3", "MP3"), (".json", "JSON"), (".png", "PNG"), (".mp4", "MP4")] {
        let path = if ext == ".mp3" {
            song_dir.join(&filename)
        } else {
            song_dir.join(format!("{}{}", base, ext))
        };
        if path.exists() && fs::remove_file(&path).is_ok() {
            deleted.push(label.to_string());
        }
    }

    if deleted.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No files found"})),
        )
            .into_response();
    }

    crate::sync_m3u_playlist(&song_dir);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": format!("Deleted: {}", deleted.join(", ")),
            "deleted_files": deleted,
        })),
    )
        .into_response()
}

// ─── Song Move ─────────────────────────────────────────────────────────

pub async fn move_song(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    headers: HeaderMap,
    Json(req): Json<MoveSongRequest>,
) -> impl IntoResponse {
    if check_auth(&state, &headers).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let filename = if filename.ends_with(".mp3") {
        filename
    } else {
        format!("{}.mp3", filename)
    };
    let base = filename.trim_end_matches(".mp3");

    let output_dir = PathBuf::from(&state.output_dir);
    let accepted_dir = PathBuf::from(&state.accepted_dir);

    let (source_dir, target_dir) = match (
        req.target.as_str(),
        output_dir.join(&filename).exists(),
        accepted_dir.join(&filename).exists(),
    ) {
        ("accepted", true, false) => (output_dir.clone(), accepted_dir.clone()),
        ("output", false, true) => (accepted_dir.clone(), output_dir.clone()),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Song not found or already in target"})),
            )
                .into_response();
        }
    };

    fs::create_dir_all(&target_dir).unwrap_or_else(|_| ());

    let mut moved = Vec::new();
    for (ext, label) in [(".mp3", "MP3"), (".json", "JSON"), (".png", "PNG"), (".mp4", "MP4")] {
        let src = if ext == ".mp3" {
            source_dir.join(&filename)
        } else {
            source_dir.join(format!("{}{}", base, ext))
        };
        let tgt = if ext == ".mp3" {
            target_dir.join(&filename)
        } else {
            target_dir.join(format!("{}{}", base, ext))
        };
        if src.exists() && fs::rename(&src, &tgt).is_ok() {
            moved.push(label.to_string());
        }
    }

    if moved.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to move files"})),
        )
            .into_response();
    }

    crate::sync_m3u_playlist(&source_dir);
    crate::sync_m3u_playlist(&target_dir);

    let target_name = if target_dir == accepted_dir {
        "accepted"
    } else {
        "output"
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": format!("Moved to {}. Files: {}", target_name, moved.join(", ")),
            "moved_files": moved,
        })),
    )
        .into_response()
}
