use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use http::HeaderMap;
use serde::Serialize;
use tracing::warn;

use common::domain::TenantSlug;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tenants/{slug}/members", get(list_members))
}

#[derive(Serialize)]
struct MemberView {
    email: String,
    display_name: Option<String>,
    role: String,
    joined_at: String,
    /// Placeholder until per-period status is wired through. Frontend
    /// already renders "—" as the neutral fallback.
    last_period_status: String,
}

#[derive(Serialize)]
struct MembersResponse {
    ok: bool,
    members: Vec<MemberView>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
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

async fn list_members(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<MembersResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "members auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Any member of the venture can read the member list — same auth model
    // as `GET /api/tenants/:slug/settings`.
    let memberships = state
        .ledger
        .list_memberships_for(&claims.email)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_memberships_for failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    if !memberships.iter().any(|m| m.tenant_slug == slug) {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let rows = state
        .ledger
        .list_tenant_members(tenant_slug.as_str())
        .await
        .map_err(|e| {
            warn!(error = %e, "list_tenant_members failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(MembersResponse {
        ok: true,
        members: rows
            .into_iter()
            .map(|m| MemberView {
                email: m.email,
                display_name: m.display_name,
                role: m.role,
                joined_at: m.joined_at.to_rfc3339(),
                last_period_status: "—".into(),
            })
            .collect(),
    }))
}
