use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::{Role, TenantSlug};
use ledger::{InviteRecord, NewInvite, RecordIdKey, UpsertUser};

use crate::mailer::InviteEmail;
use crate::state::AppState;

/// Invite admin endpoints. All require the caller to be an `owner` of the
/// venture — treasurers and members can't see or mutate invites.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tenants/{slug}/invites", post(create_invite))
        .route("/api/tenants/{slug}/invites", get(list_invites))
        .route("/api/tenants/{slug}/invites/{key}", delete(revoke_invite))
        .route(
            "/api/tenants/{slug}/invites/{key}/permanent",
            delete(delete_invite_permanent),
        )
}

#[derive(Deserialize)]
struct CreateInviteRequest {
    email: String,
    #[serde(default = "default_role")]
    role: Role,
}

fn default_role() -> Role {
    Role::Member
}

#[derive(Serialize)]
struct InviteView {
    id: String,
    tenant_slug: String,
    email: String,
    role: String,
    status: &'static str,
    created_at: String,
    accepted_at: Option<String>,
    revoked_at: Option<String>,
}

#[derive(Serialize)]
struct InviteResponse {
    ok: bool,
    invite: InviteView,
}

#[derive(Serialize)]
struct InvitesResponse {
    ok: bool,
    invites: Vec<InviteView>,
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

fn invite_status(rec: &InviteRecord) -> &'static str {
    if rec.revoked_at.is_some() {
        "revoked"
    } else if rec.accepted_at.is_some() {
        "accepted"
    } else {
        "pending"
    }
}

fn invite_view(rec: InviteRecord) -> InviteView {
    // Default-CREATE record keys come back as `RecordIdKey::String(...)` in
    // SurrealDB 3.x. Handle the other variants defensively so a future
    // schema change (e.g. UUID ids) doesn't silently 500.
    let id = match &rec.id.key {
        RecordIdKey::String(s) => s.clone(),
        RecordIdKey::Uuid(u) => u.to_string(),
        RecordIdKey::Number(n) => n.to_string(),
        _ => format!("{:?}", rec.id.key),
    };
    let status = invite_status(&rec);
    InviteView {
        id,
        tenant_slug: rec.tenant_slug,
        email: rec.email,
        role: rec.role,
        status,
        created_at: rec.created_at.to_rfc3339(),
        accepted_at: rec.accepted_at.map(|t| t.to_rfc3339()),
        revoked_at: rec.revoked_at.map(|t| t.to_rfc3339()),
    }
}

/// Verify the caller is an owner of `slug`. Returns the verified email + the
/// caller's `RecordId` (fetched via `upsert_user`) for use in `invited_by`.
async fn require_owner(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
) -> Result<(String, ledger::UserRecord), (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "invites auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let memberships = state
        .ledger
        .list_memberships_for(&claims.email)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_memberships_for failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let role = memberships
        .into_iter()
        .find(|m| m.tenant_slug == slug)
        .map(|m| m.role);

    match role.as_deref() {
        Some("owner") => {}
        Some(_) => return Err(err(StatusCode::FORBIDDEN, "owner role required")),
        None => return Err(err(StatusCode::FORBIDDEN, "not a member of this venture")),
    }

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

    Ok((claims.email, user))
}

fn valid_email(s: &str) -> bool {
    let s = s.trim();
    match s.find('@') {
        Some(i) if i > 0 && i + 1 < s.len() => {
            let domain = &s[i + 1..];
            domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        }
        _ => false,
    }
}

async fn create_invite(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateInviteRequest>,
) -> Result<Json<InviteResponse>, (StatusCode, Json<ErrorBody>)> {
    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    let (caller_email, caller) = require_owner(&state, &headers, &slug).await?;

    let email = payload.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }

    let role_str = payload.role.as_str().to_string();
    let inviter_name = caller.display_name.clone();

    let invite = state
        .ledger
        .create_invite(NewInvite {
            tenant_slug: tenant_slug.as_str().to_string(),
            email: email.clone(),
            role: role_str.clone(),
            invited_by: caller.id,
        })
        .await
        .map_err(|e| match e {
            ledger::Error::InviteExists { .. } => err(
                StatusCode::CONFLICT,
                "an active invite already exists for this email",
            ),
            other => {
                warn!(error = %other, "create_invite failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, by = %caller_email, invitee = %email, "invite created");

    // Fire-and-forget invite email. We deliberately don't fail the HTTP
    // response when SMTP is down — the invite row is persisted and the
    // owner can resend via the admin UI. Failures are logged inside `send_invite`.
    let display_name = state
        .ledger
        .tenant_display_name(&slug)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| slug.clone());

    let mailer = state.mailer.clone();
    let invitee = email.clone();
    let slug_for_email = slug.clone();
    let inviter_email = caller_email.clone();
    tokio::spawn(async move {
        let _ = mailer
            .send_invite(InviteEmail {
                to_email: &invitee,
                tenant_slug: &slug_for_email,
                tenant_display_name: &display_name,
                role: &role_str,
                invited_by_name: inviter_name.as_deref(),
                invited_by_email: &inviter_email,
            })
            .await;
    });

    Ok(Json(InviteResponse {
        ok: true,
        invite: invite_view(invite),
    }))
}

async fn list_invites(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<InvitesResponse>, (StatusCode, Json<ErrorBody>)> {
    let _ =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    require_owner(&state, &headers, &slug).await?;

    let rows = state
        .ledger
        .list_invites_for_tenant(&slug)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_invites_for_tenant failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(InvitesResponse {
        ok: true,
        invites: rows.into_iter().map(invite_view).collect(),
    }))
}

async fn delete_invite_permanent(
    State(state): State<AppState>,
    Path((slug, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let _ =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    if key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invite key required"));
    }

    let (caller_email, _) = require_owner(&state, &headers, &slug).await?;

    state
        .ledger
        .delete_invite(&slug, &key)
        .await
        .map_err(|e| match e {
            ledger::Error::NotFound => err(
                StatusCode::NOT_FOUND,
                "invite not found, already accepted, or in another tenant",
            ),
            other => {
                warn!(error = %other, "delete_invite failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, by = %caller_email, key = %key, "invite deleted");
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn revoke_invite(
    State(state): State<AppState>,
    Path((slug, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<InviteResponse>, (StatusCode, Json<ErrorBody>)> {
    let _ =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    if key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invite key required"));
    }

    let (caller_email, _) = require_owner(&state, &headers, &slug).await?;

    let invite = state
        .ledger
        .revoke_invite(&slug, &key)
        .await
        .map_err(|e| match e {
            ledger::Error::NotFound => {
                err(StatusCode::NOT_FOUND, "invite not found or already revoked")
            }
            other => {
                warn!(error = %other, "revoke_invite failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    info!(slug = %slug, by = %caller_email, key = %key, "invite revoked");
    Ok(Json(InviteResponse {
        ok: true,
        invite: invite_view(invite),
    }))
}
