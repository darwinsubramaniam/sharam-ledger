use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use ledger::UpsertUser;

use crate::state::AppState;

#[derive(Deserialize)]
struct GoogleRequest {
    credential: String,
}

#[derive(Serialize)]
struct GoogleResponse {
    ok: bool,
    user: AuthUser,
    /// Slugs of pending invites that were materialized into memberships
    /// during this sign-in. Frontend can surface a "you joined X" toast.
    accepted_invites: Vec<String>,
}

#[derive(Serialize)]
struct AuthUser {
    sub: String,
    email: String,
    name: Option<String>,
    picture: Option<String>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/auth/google", post(google))
}

async fn google(
    State(state): State<AppState>,
    Json(payload): Json<GoogleRequest>,
) -> Result<Json<GoogleResponse>, (StatusCode, Json<ErrorBody>)> {
    let claims = match state.google.verify(&payload.credential).await {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "google sign-in verification failed");
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    ok: false,
                    error: e.to_string(),
                }),
            ));
        }
    };

    info!(sub = %claims.sub, email = %claims.email, "google sign-in verified");

    // Upsert into the control-plane `user` table so we have a stable
    // RecordId to attach memberships to.
    let user = state
        .ledger
        .upsert_user(UpsertUser {
            email: claims.email.clone(),
            google_sub: claims.sub.clone(),
            display_name: claims.name.clone(),
        })
        .await
        .map_err(|e| {
            warn!(error = %e, "upsert_user failed during sign-in");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    ok: false,
                    error: e.to_string(),
                }),
            )
        })?;

    // Materialize any pending invites for this email. Idempotent — safe to
    // run on every sign-in.
    let accepted_invites = state
        .ledger
        .accept_pending_invites(&claims.email, user.id.clone())
        .await
        .map_err(|e| {
            warn!(error = %e, "accept_pending_invites failed during sign-in");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    ok: false,
                    error: e.to_string(),
                }),
            )
        })?;
    if !accepted_invites.is_empty() {
        info!(
            email = %claims.email,
            ?accepted_invites,
            "materialized pending invites into memberships"
        );
    }

    Ok(Json(GoogleResponse {
        ok: true,
        user: AuthUser {
            sub: claims.sub,
            email: claims.email,
            name: claims.name,
            picture: claims.picture,
        },
        accepted_invites,
    }))
}
