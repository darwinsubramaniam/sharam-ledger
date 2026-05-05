use std::fmt;

use dioxus::document;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    NotSignedIn,
    Unauthorized,
    Conflict(String),
    BadRequest(String),
    Other(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotSignedIn => f.write_str("Not signed in. Please sign in first."),
            ApiError::Unauthorized => f.write_str("Your session expired. Please sign in again."),
            ApiError::Conflict(m) | ApiError::BadRequest(m) | ApiError::Other(m) => f.write_str(m),
        }
    }
}

/// Storage key for the Sharam session JWT (HS256). Despite the legacy name
/// (`sharam_id_token` was once the raw Google ID token), this now holds the
/// gateway-minted session token regardless of sign-in method.
pub const TOKEN_KEY: &str = "sharam_id_token";

pub fn read_token() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(TOKEN_KEY)
        .ok()?
}

pub fn store_token(token: &str) -> Result<(), ApiError> {
    let storage = web_sys::window()
        .ok_or_else(|| ApiError::Other("no window".into()))?
        .local_storage()
        .map_err(|_| ApiError::Other("localStorage unavailable".into()))?
        .ok_or_else(|| ApiError::Other("localStorage unavailable".into()))?;
    storage
        .set_item(TOKEN_KEY, token)
        .map_err(|_| ApiError::Other("localStorage write failed".into()))
}

pub fn api_url(path: &str) -> Result<String, ApiError> {
    let origin = web_sys::window()
        .ok_or_else(|| ApiError::Other("no window".into()))?
        .location()
        .origin()
        .map_err(|_| ApiError::Other("no origin".into()))?;
    Ok(format!("{origin}{path}"))
}

/// Build a request with the Authorization header set to the stored Sharam
/// session JWT. Errors if the token is missing.
pub fn authed(method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder, ApiError> {
    let Some(token) = read_token() else {
        return Err(ApiError::NotSignedIn);
    };
    let url = api_url(path)?;
    Ok(reqwest::Client::new()
        .request(method, &url)
        .header("Authorization", format!("Bearer {token}")))
}

/// Public, deployment-time config the gateway exposes to the wasm bundle so
/// the same image can run against any Google OAuth client. Fetched once on
/// app boot via `GET /api/config`.
#[derive(Debug, Clone, Deserialize)]
pub struct PublicConfig {
    pub google_client_id: String,
}

pub async fn fetch_public_config() -> Result<PublicConfig, ApiError> {
    let url = api_url("/api/config")?;
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("network: {e}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    resp.json::<PublicConfig>()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))
}

/// Decoded subset of Google ID-token claims we actually use.
/// The token is verified server-side on every protected request, so
/// client-side decoding here is purely for display.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserClaims {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub picture: String,
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub iat: i64,
    #[serde(default)]
    pub exp: i64,
}

/// Decode the stored JWT's payload claims. Returns `NotSignedIn` if the
/// token is missing.
pub fn current_user() -> Result<UserClaims, ApiError> {
    let token = read_token().ok_or(ApiError::NotSignedIn)?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ApiError::Other("malformed token".into()));
    }
    // base64url → standard base64 with padding
    let mut b64 = parts[1].replace('-', "+").replace('_', "/");
    while !b64.len().is_multiple_of(4) {
        b64.push('=');
    }
    let json = web_sys::window()
        .ok_or_else(|| ApiError::Other("no window".into()))?
        .atob(&b64)
        .map_err(|_| ApiError::Other("base64 decode failed".into()))?;
    serde_json::from_str::<UserClaims>(&json)
        .map_err(|e| ApiError::Other(format!("claim decode: {e}")))
}

/// Clear the stored token, tell Google Identity Services to drop its
/// auto-select hint (so the next visit to /login actually prompts the
/// account chooser instead of silently re-signing the same identity), and
/// navigate to /login.
pub fn sign_out() {
    // Disable Google's auto-select. Wrapped in try/catch because the GIS
    // script may not be loaded on every page (e.g. dashboard) — that's fine.
    let _ = document::eval(
        "try { \
            if (window.google && google.accounts && google.accounts.id) { \
                google.accounts.id.disableAutoSelect(); \
            } \
        } catch (_) {}",
    );

    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.remove_item(TOKEN_KEY);
        }
        let _ = window.location().set_href("/login");
    }
}

// ─── auth endpoints ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AuthSuccess {
    pub token: String,
    /// Slugs the gateway just promoted from invite → membership for this
    /// caller. Kept around so a future toast can announce them.
    #[serde(default)]
    #[allow(dead_code)]
    pub accepted_invites: Vec<String>,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
struct RegisterRequest<'a> {
    email: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
}

async fn post_auth<B: Serialize>(path: &str, body: &B) -> Result<AuthSuccess, ApiError> {
    let url = api_url(path)?;
    let resp = reqwest::Client::new()
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("network: {e}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    resp.json::<AuthSuccess>()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))
}

pub async fn login_with_password(email: &str, password: &str) -> Result<AuthSuccess, ApiError> {
    post_auth("/api/auth/login", &LoginRequest { email, password }).await
}

pub async fn register_with_password(
    email: &str,
    password: &str,
    display_name: Option<&str>,
) -> Result<AuthSuccess, ApiError> {
    post_auth(
        "/api/auth/register",
        &RegisterRequest {
            email,
            password,
            display_name,
        },
    )
    .await
}

/// Map a non-success response into a typed error using the gateway's
/// `{ok:false, error:"..."}` envelope where present.
pub async fn into_api_error(resp: reqwest::Response) -> ApiError {
    let status = resp.status();
    let body_msg: Option<String> = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from));
    let msg = body_msg.unwrap_or_else(|| format!("HTTP {status}"));
    match status.as_u16() {
        401 => ApiError::Unauthorized,
        400 => ApiError::BadRequest(msg),
        409 => ApiError::Conflict(msg),
        _ => ApiError::Other(msg),
    }
}
