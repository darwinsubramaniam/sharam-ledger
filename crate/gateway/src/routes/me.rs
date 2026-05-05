use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use http::HeaderMap;
use serde::Serialize;
use tracing::warn;

use crate::state::AppState;

#[derive(Serialize)]
struct VentureView {
    slug: String,
    display_name: String,
    role: String,
    created_at: String,
}

#[derive(Serialize)]
struct VenturesResponse {
    ok: bool,
    ventures: Vec<VentureView>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/me/ventures", get(list_my_ventures))
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
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

async fn list_my_ventures(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<VenturesResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "ventures auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let ventures = state
        .ledger
        .list_user_ventures(&identity.email)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_user_ventures failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(VenturesResponse {
        ok: true,
        ventures: ventures
            .into_iter()
            .map(|v| VentureView {
                slug: v.slug,
                display_name: v.display_name,
                role: v.role,
                created_at: v.created_at.to_rfc3339(),
            })
            .collect(),
    }))
}
