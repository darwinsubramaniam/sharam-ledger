use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::TenantSlug;
use ledger::{CarryForwardRecord, NewCarryForward};

use crate::state::AppState;

/// Per-tenant carry-forward seed: money the venture had collected off-platform
/// before migrating to Sharam. Set ONCE by the owner at venture creation;
/// immutable thereafter (DB-side EVENT enforces this). Any member can read.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tenants/{slug}/carry-forward", get(get_carry_forward))
        .route("/api/tenants/{slug}/carry-forward", post(set_carry_forward))
}

#[derive(Serialize)]
struct CarryForwardView {
    from_date: String,
    to_date: String,
    amount_cents: i64,
    note: Option<String>,
    recorded_by: String,
    recorded_at: String,
}

fn view(rec: CarryForwardRecord) -> CarryForwardView {
    CarryForwardView {
        from_date: rec.from_date,
        to_date: rec.to_date,
        amount_cents: rec.amount_cents,
        note: rec.note,
        recorded_by: rec.recorded_by,
        recorded_at: rec.recorded_at.to_rfc3339(),
    }
}

#[derive(Serialize)]
struct GetResponse {
    ok: bool,
    carry_forward: Option<CarryForwardView>,
}

#[derive(Deserialize)]
struct SetRequest {
    from_date: String,
    to_date: String,
    amount_cents: i64,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Serialize)]
struct SetResponse {
    ok: bool,
    carry_forward: CarryForwardView,
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

async fn caller_role(
    state: &AppState,
    email: &str,
    slug: &str,
) -> Result<Option<String>, (StatusCode, Json<ErrorBody>)> {
    let memberships = state
        .ledger
        .list_memberships_for(email)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_memberships_for failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    Ok(memberships
        .into_iter()
        .find(|m| m.tenant_slug == slug)
        .map(|m| m.role))
}

// ISO calendar date YYYY-MM-DD. The schema regex is the backstop — this is
// a fail-fast sanity check at the wire boundary so the client gets a 400
// rather than a 500 wrapping a SurrealDB ASSERT throw.
fn is_iso_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    s.chars()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

async fn get_carry_forward(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<GetResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "carry-forward get auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if caller_role(&state, &identity.email, &slug).await?.is_none() {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let row = state
        .ledger
        .get_carry_forward(&tenant_slug)
        .await
        .map_err(|e| {
            warn!(error = %e, "get_carry_forward failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(GetResponse {
        ok: true,
        carry_forward: row.map(view),
    }))
}

async fn set_carry_forward(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SetRequest>,
) -> Result<Json<SetResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "carry-forward set auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Owner-only — carry-forward is a venture-wide config decision, same as
    // dues amount / cadence.
    match caller_role(&state, &identity.email, &slug).await? {
        Some(r) if r == "owner" => {}
        Some(_) => return Err(err(StatusCode::FORBIDDEN, "owner role required")),
        None => return Err(err(StatusCode::FORBIDDEN, "not a member of this venture")),
    }

    if !is_iso_date(&payload.from_date) || !is_iso_date(&payload.to_date) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "from_date and to_date must be ISO calendar dates (YYYY-MM-DD)",
        ));
    }
    if payload.from_date > payload.to_date {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "from_date must be on or before to_date",
        ));
    }
    if payload.amount_cents <= 0 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "amount_cents must be greater than 0",
        ));
    }
    let note = payload.note.and_then(|n| {
        let trimmed = n.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let row = state
        .ledger
        .set_carry_forward(
            &tenant_slug,
            NewCarryForward {
                from_date: payload.from_date,
                to_date: payload.to_date,
                amount_cents: payload.amount_cents,
                note,
                recorded_by: identity.email.clone(),
            },
        )
        .await
        .map_err(|e| match e {
            ledger::Error::CarryForwardExists => err(
                StatusCode::CONFLICT,
                "carry-forward seed is already set for this venture",
            ),
            other => {
                warn!(error = %other, "set_carry_forward failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, by = %identity.email, "carry-forward seeded");
    Ok(Json(SetResponse {
        ok: true,
        carry_forward: view(row),
    }))
}
