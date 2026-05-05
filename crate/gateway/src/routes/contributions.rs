use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use common::domain::{Cadence, Period, TenantSlug, parse_tz};
use ledger::{
    AccumulationPoint, ContributionRecord, NewContribution, PeriodSummary, RecordIdKey,
    TenantSettings,
};

use crate::state::AppState;

/// Per-tenant contribution endpoints. Any member of the venture may submit
/// payments for themselves and read their own period roll-up.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tenants/{slug}/contributions", post(post_contribution))
        .route(
            "/api/tenants/{slug}/contributions/me",
            get(my_contributions),
        )
        .route(
            "/api/tenants/{slug}/contributions/pool",
            get(pool_contributions),
        )
        .route("/api/tenants/{slug}/periods", get(active_periods))
        .route("/api/tenants/{slug}/accumulation", get(accumulation))
}

// ─── wire shapes ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PostContributionRequest {
    amount_cents: i64,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    proof_key: Option<String>,
}

#[derive(Serialize)]
struct ContributionView {
    id: String,
    user_email: String,
    cadence: String,
    period: String,
    amount_cents: i64,
    status: String,
    note: Option<String>,
    proof_key: Option<String>,
    submitted_at: DateTime<Utc>,
}

fn contribution_view(r: ContributionRecord) -> ContributionView {
    // RecordId has no Display impl in surrealdb 3.x — render the key portion
    // (the part after `contribution:`). Frontend uses it for list keys only.
    let id = match &r.id.key {
        RecordIdKey::String(s) => s.clone(),
        RecordIdKey::Uuid(u) => u.to_string(),
        RecordIdKey::Number(n) => n.to_string(),
        _ => format!("{:?}", r.id.key),
    };
    ContributionView {
        id,
        user_email: r.user_email,
        cadence: r.cadence,
        period: r.period,
        amount_cents: r.amount_cents,
        status: r.status,
        note: r.note,
        proof_key: r.proof_key,
        submitted_at: r.submitted_at,
    }
}

#[derive(Serialize)]
struct PeriodSummaryView {
    period: String,
    cadence: String,
    currency: String,
    dues_cents: i64,
    paid_cents: i64,
    remaining_cents: i64,
}

#[derive(Serialize)]
struct PostContributionResponse {
    ok: bool,
    contribution: ContributionView,
    summary: PeriodSummaryView,
}

#[derive(Serialize)]
struct ListContributionsResponse {
    ok: bool,
    summary: PeriodSummaryView,
    contributions: Vec<ContributionView>,
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    period: Option<String>,
}

#[derive(Serialize)]
struct PoolSummaryView {
    period: String,
    cadence: String,
    currency: String,
    dues_per_member_cents: i64,
    member_count: usize,
    target_cents: i64,
    paid_cents: i64,
    remaining_cents: i64,
    settled_count: usize,
}

#[derive(Serialize)]
struct PoolResponse {
    ok: bool,
    summary: PoolSummaryView,
    contributions: Vec<ContributionView>,
}

#[derive(Serialize)]
struct PeriodsResponse {
    ok: bool,
    current_period: String,
    cadence: String,
    periods: Vec<String>,
}

#[derive(Deserialize)]
struct AccumulationQuery {
    /// "auto" | "year" | "month". Defaults to "auto" — yearly buckets if
    /// the venture spans ≥ 12 months of activity, monthly otherwise.
    #[serde(default)]
    bucket: Option<String>,
}

#[derive(Serialize)]
struct AccumulationPointView {
    bucket: String,
    period_cents: i64,
    cumulative_cents: i64,
}

#[derive(Serialize)]
struct AccumulationResponse {
    ok: bool,
    bucket: String,
    currency: String,
    series: Vec<AccumulationPointView>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

// ─── helpers ───────────────────────────────────────────────────────────────

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

/// Parses cadence + timezone from settings and returns the period string
/// the tenant considers "current" right now.
fn current_period_for(s: &TenantSettings) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    let tz =
        parse_tz(&s.timezone).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cadence: Cadence = s
        .cadence
        .parse()
        .map_err(|e: common::error::Error| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Period::current_in(tz, cadence).to_string())
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

async fn settings_for(
    state: &AppState,
    slug: &TenantSlug,
) -> Result<TenantSettings, (StatusCode, Json<ErrorBody>)> {
    state
        .ledger
        .tenant_settings(slug)
        .await
        .map_err(|e| match e {
            ledger::Error::NotFound => err(StatusCode::NOT_FOUND, "settings not found"),
            other => {
                warn!(error = %other, "tenant_settings failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })
}

// ─── handlers ──────────────────────────────────────────────────────────────

async fn post_contribution(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PostContributionRequest>,
) -> Result<Json<PostContributionResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "contribution post auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &claims.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    if payload.amount_cents <= 0 {
        return Err(err(StatusCode::BAD_REQUEST, "amount_cents must be > 0"));
    }

    // Read settings → derive cadence + current period. Members never write
    // for arbitrary periods — only "right now" — so the client does not pass
    // `period`. The DB-side lock would catch a stale period anyway.
    let settings = settings_for(&state, &tenant_slug).await?;
    let period = current_period_for(&settings)?;
    let cadence = settings.cadence.clone();

    let row = state
        .ledger
        .add_contribution(
            &tenant_slug,
            NewContribution {
                user_email: claims.email.clone(),
                cadence: cadence.clone(),
                period: period.clone(),
                amount_cents: payload.amount_cents,
                proof_key: payload.proof_key,
                note: payload.note,
            },
        )
        .await
        .map_err(|e| match e {
            ledger::Error::PeriodLocked { period } => err(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("period {period} is locked"),
            ),
            ledger::Error::DuesCapExceeded {
                paid_cents,
                dues_cents,
            } => err(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("dues cap exceeded: would total {paid_cents} cents (cap {dues_cents})"),
            ),
            other => {
                warn!(error = %other, "add_contribution failed");
                err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        })?;

    let summary = state
        .ledger
        .period_summary(&tenant_slug, &claims.email, &period)
        .await
        .map_err(|e| {
            warn!(error = %e, "period_summary failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    info!(
        slug = %slug,
        by = %claims.email,
        period = %period,
        amount = payload.amount_cents,
        "contribution recorded"
    );

    Ok(Json(PostContributionResponse {
        ok: true,
        contribution: contribution_view(row),
        summary: summary_view(period, cadence, settings.currency, summary),
    }))
}

async fn my_contributions(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Result<Json<ListContributionsResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "contribution list auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &claims.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let settings = settings_for(&state, &tenant_slug).await?;
    let period = match q.period {
        Some(p) => {
            // Validate the shape — guards against open queries with garbage.
            p.parse::<Period>()
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("invalid period: {e}")))?;
            p
        }
        None => current_period_for(&settings)?,
    };

    let rows = state
        .ledger
        .list_contributions(&tenant_slug, &claims.email, &period)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_contributions failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let summary = state
        .ledger
        .period_summary(&tenant_slug, &claims.email, &period)
        .await
        .map_err(|e| {
            warn!(error = %e, "period_summary failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(ListContributionsResponse {
        ok: true,
        summary: summary_view(period, settings.cadence, settings.currency, summary),
        contributions: rows.into_iter().map(contribution_view).collect(),
    }))
}

fn summary_view(
    period: String,
    cadence: String,
    currency: String,
    s: PeriodSummary,
) -> PeriodSummaryView {
    PeriodSummaryView {
        period,
        cadence,
        currency,
        dues_cents: s.dues_cents,
        paid_cents: s.paid_cents,
        remaining_cents: s.remaining_cents,
    }
}

async fn pool_contributions(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Result<Json<PoolResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "pool list auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &claims.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let settings = settings_for(&state, &tenant_slug).await?;
    let period = match q.period {
        Some(p) => {
            p.parse::<Period>()
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("invalid period: {e}")))?;
            p
        }
        None => current_period_for(&settings)?,
    };

    let rows = state
        .ledger
        .list_pool_contributions(&tenant_slug, &period)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_pool_contributions failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let members = state
        .ledger
        .list_tenant_members(&slug)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_tenant_members failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let dues_per_member_cents = settings.dues_amount_cents;
    let member_count = members.len();
    let target_cents = dues_per_member_cents.saturating_mul(member_count as i64);

    // Per-member non-rejected sums for `settled_count` and total `paid_cents`.
    let mut by_member: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in &rows {
        if r.status != "rejected" {
            *by_member.entry(r.user_email.clone()).or_default() += r.amount_cents;
        }
    }
    let paid_cents: i64 = by_member.values().sum();
    let settled_count = if dues_per_member_cents > 0 {
        by_member
            .values()
            .filter(|v| **v >= dues_per_member_cents)
            .count()
    } else {
        // Donation-style venture (no cap) — "settled" has no meaning; report
        // the count of members who contributed at all so the UI can still
        // show "X / Y participating".
        by_member.values().filter(|v| **v > 0).count()
    };
    let remaining_cents = (target_cents - paid_cents).max(0);

    Ok(Json(PoolResponse {
        ok: true,
        summary: PoolSummaryView {
            period,
            cadence: settings.cadence,
            currency: settings.currency,
            dues_per_member_cents,
            member_count,
            target_cents,
            paid_cents,
            remaining_cents,
            settled_count,
        },
        contributions: rows.into_iter().map(contribution_view).collect(),
    }))
}

async fn active_periods(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<PeriodsResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "periods auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &claims.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let settings = settings_for(&state, &tenant_slug).await?;
    let current_period = current_period_for(&settings)?;
    let mut periods = state
        .ledger
        .list_active_periods(&tenant_slug)
        .await
        .map_err(|e| {
            warn!(error = %e, "list_active_periods failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    // Always include the current period in the dropdown — even if no row
    // has been written yet — so members can submit and view "this period".
    if !periods.iter().any(|p| p == &current_period) {
        periods.insert(0, current_period.clone());
    }

    Ok(Json(PeriodsResponse {
        ok: true,
        current_period,
        cadence: settings.cadence,
        periods,
    }))
}

async fn accumulation(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<AccumulationQuery>,
    headers: HeaderMap,
) -> Result<Json<AccumulationResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let claims = state.google.verify(token).await.map_err(|e| {
        warn!(error = %e, "accumulation auth failed");
        err(StatusCode::UNAUTHORIZED, e.to_string())
    })?;

    let tenant_slug =
        TenantSlug::new(slug.as_str()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if !caller_is_member(&state, &claims.email, &slug).await? {
        return Err(err(StatusCode::FORBIDDEN, "not a member of this venture"));
    }

    let settings = settings_for(&state, &tenant_slug).await?;
    let points = state
        .ledger
        .accumulation_points(&tenant_slug)
        .await
        .map_err(|e| {
            warn!(error = %e, "accumulation_points failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let bucket_mode = match q.bucket.as_deref().unwrap_or("auto") {
        "year" => "year",
        "month" => "month",
        _ => decide_bucket(&points),
    };

    let series = bucket_series(&points, bucket_mode);

    Ok(Json(AccumulationResponse {
        ok: true,
        bucket: bucket_mode.to_string(),
        currency: settings.currency,
        series,
    }))
}

/// Auto bucket picker: yearly when the venture's first non-rejected payment
/// is at least 12 months old (so multiple yearly bars are meaningful);
/// monthly otherwise. Empty venture → monthly so the empty chart still has
/// a reasonable axis label format.
fn decide_bucket(points: &[AccumulationPoint]) -> &'static str {
    use chrono::Duration;
    let Some(first) = points.first() else {
        return "month";
    };
    let span = Utc::now().signed_duration_since(first.submitted_at);
    if span >= Duration::days(365) {
        "year"
    } else {
        "month"
    }
}

/// Group points into buckets keyed by year ("YYYY") or year-month ("YYYY-MM"),
/// then walk in chronological order computing the running cumulative total.
fn bucket_series(points: &[AccumulationPoint], mode: &str) -> Vec<AccumulationPointView> {
    use chrono::Datelike;
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<String, i64> = BTreeMap::new();
    for p in points {
        let key = match mode {
            "year" => format!("{:04}", p.submitted_at.year()),
            _ => format!("{:04}-{:02}", p.submitted_at.year(), p.submitted_at.month()),
        };
        *buckets.entry(key).or_default() += p.amount_cents;
    }
    let mut running: i64 = 0;
    buckets
        .into_iter()
        .map(|(bucket, cents)| {
            running += cents;
            AccumulationPointView {
                bucket,
                period_cents: cents,
                cumulative_cents: running,
            }
        })
        .collect()
}
