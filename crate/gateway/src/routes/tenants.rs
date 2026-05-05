use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::{Cadence, TenantSlug};
use ledger::{NewTenant, UpsertUser};

use crate::state::AppState;

#[derive(Deserialize)]
struct CreateTenantRequest {
    slug: String,
    display_name: String,
    timezone: String,
    currency: String,
    #[serde(default = "default_cadence")]
    cadence: Cadence,
    #[serde(default)]
    dues_amount_cents: i64,
}

fn default_cadence() -> Cadence {
    Cadence::Monthly
}

#[derive(Serialize)]
struct CreateTenantResponse {
    ok: bool,
    slug: String,
    display_name: String,
    role: &'static str,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tenants", post(create_tenant))
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

async fn create_tenant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTenantRequest>,
) -> Result<Json<CreateTenantResponse>, (StatusCode, Json<ErrorBody>)> {
    // 1. Auth — verify the Google ID token from the Authorization header.
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "tenant create auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    // 2. Validate slug against the schema regex.
    let slug = TenantSlug::new(payload.slug.trim())
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    let display_name = payload.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "display_name is required"));
    }

    if payload.dues_amount_cents < 0 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "dues_amount_cents must be >= 0",
        ));
    }

    // 3. Upsert the caller into the control-plane `user` table.
    let user = state
        .ledger
        .upsert_user(UpsertUser {
            email: claims.email.clone(),
            google_sub: claims.sub.clone(),
            display_name: claims.name.clone(),
        })
        .await
        .map_err(|e| {
            warn!(error = %e, "upsert_user failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    // 4. Provision the tenant. Ledger creates the namespace, applies the
    //    tenant schema, registers it in `tenant_directory`, and writes the
    //    first owner-membership — all in one call.
    state
        .ledger
        .create_tenant(NewTenant {
            slug: slug.clone(),
            display_name: display_name.clone(),
            timezone: payload.timezone,
            currency: payload.currency,
            cadence: payload.cadence.as_str().to_string(),
            dues_amount_cents: payload.dues_amount_cents,
            created_by: user.id,
        })
        .await
        .map_err(|e| match e {
            ledger::Error::TenantExists(s) => {
                err(StatusCode::CONFLICT, format!("tenant '{s}' already exists"))
            }
            other => {
                warn!(error = %other, "create_tenant failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, owner = %claims.email, "tenant created");
    Ok(Json(CreateTenantResponse {
        ok: true,
        slug: slug.into_inner(),
        display_name,
        role: "owner",
    }))
}
