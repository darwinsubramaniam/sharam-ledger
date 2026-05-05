use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use auth::Identity;
use ledger::{RegisterPassword, UpsertUser};

use crate::state::AppState;

// ─── shared response shapes ────────────────────────────────────────────────

#[derive(Serialize)]
struct AuthUser {
    /// Stable identifier — the user's email. Older clients keyed off the
    /// Google `sub` here; password users have no Google identifier so we
    /// expose email as the canonical id.
    sub: String,
    email: String,
    name: Option<String>,
    picture: Option<String>,
}

#[derive(Serialize)]
struct AuthResponse {
    ok: bool,
    /// Sharam-issued session JWT (HS256). The client stores this and
    /// attaches it as `Authorization: Bearer <token>` on every protected
    /// request. Unlike the previous Google ID token, this is fully under
    /// our control — same shape regardless of sign-in method.
    token: String,
    user: AuthUser,
    /// Slugs of pending invites that were materialized into memberships
    /// during this sign-in. Frontend can surface a "you joined X" toast.
    #[serde(default)]
    accepted_invites: Vec<String>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (
        status,
        Json(ErrorBody {
            ok: false,
            error: msg.into(),
        }),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/google", post(google))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
}

// ─── Google sign-in ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GoogleRequest {
    credential: String,
}

async fn google(
    State(state): State<AppState>,
    Json(payload): Json<GoogleRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorBody>)> {
    let claims = state.google.verify(&payload.credential).await.map_err(|e| {
        warn!(error = %e, "google sign-in verification failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    info!(sub = %claims.sub, email = %claims.email, "google sign-in verified");

    let user = state
        .ledger
        .upsert_user(UpsertUser {
            email: claims.email.clone(),
            google_sub: claims.sub.clone(),
            display_name: claims.name.clone(),
        })
        .await
        .map_err(|e| {
            warn!(error = %e, "upsert_user failed during google sign-in");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let accepted_invites = state
        .ledger
        .accept_pending_invites(&claims.email, user.id.clone())
        .await
        .map_err(|e| {
            warn!(error = %e, "accept_pending_invites failed during google sign-in");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    if !accepted_invites.is_empty() {
        info!(
            email = %claims.email,
            ?accepted_invites,
            "materialized pending invites into memberships"
        );
    }

    let identity = Identity {
        email: claims.email.clone(),
        name: claims.name.clone(),
        picture: claims.picture.clone(),
    };
    let token = state.sessions.issue(&identity).map_err(|e| {
        warn!(error = %e, "session issue failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(AuthResponse {
        ok: true,
        token,
        user: AuthUser {
            sub: claims.email.clone(),
            email: claims.email,
            name: claims.name,
            picture: claims.picture,
        },
        accepted_invites,
    }))
}

// ─── Email + password ──────────────────────────────────────────────────────

/// Minimum password length enforced at the gateway. Argon2 has no upper
/// bound that matters in practice, but rejecting trivially-short passwords
/// here means the UI can match the rule without round-tripping.
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

fn normalize_email(s: &str) -> String {
    s.trim().to_lowercase()
}

fn valid_email(s: &str) -> bool {
    match s.find('@') {
        Some(i) if i > 0 && i + 1 < s.len() => {
            let domain = &s[i + 1..];
            domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        }
        _ => false,
    }
}

async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorBody>)> {
    let email = normalize_email(&payload.email);
    if !valid_email(&email) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }
    if payload.password.len() < MIN_PASSWORD_LEN {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("password must be at least {MIN_PASSWORD_LEN} characters"),
        ));
    }
    let display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Hash off the request thread — Argon2 is intentionally CPU-heavy.
    let pwd = payload.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pwd))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("hash join: {e}")))?
        .map_err(|e| {
            warn!(error = %e, "password hash failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "hash failed")
        })?;

    let user = state
        .ledger
        .create_password_user(RegisterPassword {
            email: email.clone(),
            password_hash: hash,
            display_name: display_name.clone(),
        })
        .await
        .map_err(|e| match e {
            ledger::Error::UserExists(_) => err(
                StatusCode::CONFLICT,
                "an account with that email already exists — try signing in",
            ),
            other => {
                warn!(error = %other, "create_password_user failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    let accepted_invites = state
        .ledger
        .accept_pending_invites(&email, user.id.clone())
        .await
        .map_err(|e| {
            warn!(error = %e, "accept_pending_invites failed during register");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let identity = Identity {
        email: email.clone(),
        name: user.display_name.clone(),
        picture: None,
    };
    let token = state.sessions.issue(&identity).map_err(|e| {
        warn!(error = %e, "session issue failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    info!(email = %email, "password user registered");

    Ok(Json(AuthResponse {
        ok: true,
        token,
        user: AuthUser {
            sub: email.clone(),
            email,
            name: user.display_name,
            picture: None,
        },
        accepted_invites,
    }))
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, Json<ErrorBody>)> {
    let email = normalize_email(&payload.email);
    if email.is_empty() || payload.password.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "email and password required"));
    }

    let row = state
        .ledger
        .get_user_by_email(&email)
        .await
        .map_err(|e| {
            warn!(error = %e, "get_user_by_email failed during login");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    // Don't leak which half failed — same response for "no such email" and
    // "wrong password". Hashing always runs on a successful row so the
    // timing signal between the two paths stays small.
    let Some(user) = row else {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid email or password"));
    };
    let Some(hash) = user.password_hash.clone() else {
        // User exists but has no password set (Google-only). Tell them to
        // use Google so they can opt into adding a password later via a
        // dedicated flow rather than guessing why login is failing.
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "this account doesn't have a password — use Google sign-in",
        ));
    };

    let pwd = payload.password;
    let verify = tokio::task::spawn_blocking(move || auth::verify_password(&pwd, &hash))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("verify join: {e}")))?;
    if verify.is_err() {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid email or password"));
    }

    let accepted_invites = state
        .ledger
        .accept_pending_invites(&email, user.id.clone())
        .await
        .map_err(|e| {
            warn!(error = %e, "accept_pending_invites failed during login");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let identity = Identity {
        email: email.clone(),
        name: user.display_name.clone(),
        picture: None,
    };
    let token = state.sessions.issue(&identity).map_err(|e| {
        warn!(error = %e, "session issue failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    info!(email = %email, "password sign-in verified");

    Ok(Json(AuthResponse {
        ok: true,
        token,
        user: AuthUser {
            sub: email.clone(),
            email,
            name: user.display_name,
            picture: None,
        },
        accepted_invites,
    }))
}
