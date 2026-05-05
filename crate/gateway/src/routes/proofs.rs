use std::time::Duration;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::TenantSlug;
use storage::PROOF_KEY_PREFIX;

use crate::state::AppState;

/// Hard cap on a single proof upload. Receipts are small — keep the limit
/// tight to bound RAM use (the body is buffered into memory before the
/// S3 PUT).
pub const MAX_PROOF_BYTES: usize = 10 * 1024 * 1024;

/// TTL for the presigned download URL. Short — the URL is bearer-equivalent
/// for the duration; the browser only needs long enough to follow the link.
const PRESIGN_TTL: Duration = Duration::from_secs(300);

/// Per-tenant proof endpoints. Members upload via raw-body PUT (Content-Type
/// carries the mime); reads return a short-lived presigned URL the browser
/// can open directly against the storage backend.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/tenants/{slug}/proofs",
            post(post_proof).layer(DefaultBodyLimit::max(MAX_PROOF_BYTES)),
        )
        .route("/api/tenants/{slug}/proofs/url", get(get_proof_url))
}

#[derive(Serialize)]
struct PostProofResponse {
    ok: bool,
    proof_key: String,
    content_type: String,
    size_bytes: usize,
}

#[derive(Deserialize)]
struct ProofUrlQuery {
    key: String,
}

#[derive(Serialize)]
struct GetProofUrlResponse {
    ok: bool,
    url: String,
    expires_in_secs: u64,
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

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Map a request Content-Type to a stable file extension. Anything not
/// in this allowlist is rejected at upload time — keeps unrenderable
/// junk (.exe, .zip, …) out of the bucket.
fn ext_for(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

async fn caller_is_member(
    state: &AppState,
    email: &str,
    slug: &str,
) -> Result<bool, (StatusCode, Json<ErrorBody>)> {
    let memberships = state
        .ledger
        .list_memberships_for(email)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_memberships_for failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    Ok(memberships.iter().any(|m| m.tenant_slug == slug))
}

async fn post_proof(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<PostProofResponse>), (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "proof upload auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &identity.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    if body.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "empty body"));
    }

    let content_type = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        // Strip charset / boundary etc. — only the bare type is meaningful here.
        .map(|s| s.split(';').next().unwrap_or("").trim().to_lowercase())
        .unwrap_or_default();

    let ext = ext_for(&content_type).ok_or_else(|| {
        err(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "only image/jpeg, image/png, image/webp, application/pdf are accepted",
        )
    })?;

    let size = body.len();
    let key = state
        .proofs
        .put(
            tenant_slug.as_str(),
            &identity.email,
            &content_type,
            ext,
            body.to_vec(),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "proof put failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    info!(
        slug = %slug,
        by = %identity.email,
        bytes = size,
        content_type = %content_type,
        "proof uploaded"
    );

    Ok((
        StatusCode::CREATED,
        Json(PostProofResponse {
            ok: true,
            proof_key: key,
            content_type,
            size_bytes: size,
        }),
    ))
}

async fn get_proof_url(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ProofUrlQuery>,
    headers: HeaderMap,
) -> Result<Json<GetProofUrlResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let identity = state.sessions.verify(token).map_err(|e| {
        warn!(error = %e, "proof url auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let _tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    let expected_prefix = format!("{PROOF_KEY_PREFIX}/{slug}/");
    if !q.key.starts_with(&expected_prefix) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "key does not belong to this tenant",
        ));
    }

    if !caller_is_member(&state, &identity.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let url = state
        .proofs
        .presign_get(&q.key, PRESIGN_TTL)
        .await
        .map_err(|e| {
            warn!(error = %e, "presign failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(GetProofUrlResponse {
        ok: true,
        url,
        expires_in_secs: PRESIGN_TTL.as_secs(),
    }))
}
