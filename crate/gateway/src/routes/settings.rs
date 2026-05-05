use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::{Cadence, Period, TenantSlug, parse_tz};
use ledger::{TenantSettings, UpdateSettings};

use crate::state::AppState;

/// Mutable per-tenant settings. Owner-only — treasurers/members can read
/// but not change cadence, dues, or display name.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tenants/{slug}/settings", get(get_settings))
        .route("/api/tenants/{slug}/settings", patch(patch_settings))
}

#[derive(Serialize)]
struct SettingsView {
    display_name: String,
    timezone: String,
    currency: String,
    cadence: String,
    dues_amount_cents: i64,
    current_period: String,
}

fn settings_view(s: TenantSettings) -> SettingsView {
    let current_period = match (parse_tz(&s.timezone), s.cadence.parse::<Cadence>()) {
        (Ok(tz), Ok(cadence)) => Period::current_in(tz, cadence).to_string(),
        _ => String::new(),
    };
    SettingsView {
        display_name: s.display_name,
        timezone: s.timezone,
        currency: s.currency,
        cadence: s.cadence,
        dues_amount_cents: s.dues_amount_cents,
        current_period,
    }
}

#[derive(Serialize)]
struct SettingsResponse {
    ok: bool,
    settings: SettingsView,
}

#[derive(Deserialize)]
struct PatchSettingsRequest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    cadence: Option<Cadence>,
    #[serde(default)]
    dues_amount_cents: Option<i64>,
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

/// Returns the caller's role for `slug`, or `None` if they aren't a member.
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

async fn get_settings(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "settings get auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Any member can read.
    if caller_role(&state, &identity.email, &slug).await?.is_none() {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let s = state
        .ledger
        .tenant_settings(&tenant_slug)
        .await
        .map_err(|e| match e {
            ledger::Error::NotFound => err(StatusCode::NOT_FOUND, "settings not found"),
            other => {
                warn!(error = %other, "tenant_settings failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    Ok(Json(SettingsResponse {
        ok: true,
        settings: settings_view(s),
    }))
}

async fn patch_settings(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PatchSettingsRequest>,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "settings patch auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Owner-only mutation. Treasurers manage contributions, not venture config.
    match caller_role(&state, &identity.email, &slug).await? {
        Some(r) if r == "owner" => {}
        Some(_) => return Err(err(StatusCode::FORBIDDEN, "owner role required")),
        None => return Err(err(StatusCode::FORBIDDEN, "not a member of this venture")),
    }

    if let Some(amt) = payload.dues_amount_cents {
        if amt < 0 {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "dues_amount_cents must be >= 0",
            ));
        }
    }
    if let Some(name) = payload.display_name.as_ref() {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "display_name cannot be empty"));
        }
    }

    let patch = UpdateSettings {
        display_name: payload.display_name.map(|s| s.trim().to_string()),
        timezone: payload.timezone,
        currency: payload.currency,
        cadence: payload.cadence.map(|c| c.as_str().to_string()),
        dues_amount_cents: payload.dues_amount_cents,
    };

    let s = state
        .ledger
        .update_settings(&tenant_slug, patch)
        .await
        .map_err(|e| match e {
            ledger::Error::NotFound => err(StatusCode::NOT_FOUND, "settings not found"),
            other => {
                warn!(error = %other, "update_settings failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, by = %identity.email, "settings updated");
    Ok(Json(SettingsResponse {
        ok: true,
        settings: settings_view(s),
    }))
}
