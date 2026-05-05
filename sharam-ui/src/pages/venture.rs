use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::{authed, into_api_error, ApiError};
use crate::pages::sidenav::Sidenav;

// ─── Wire shape ────────────────────────────────────────────────────────────
//
// The dashboard hits /api/me/ventures for the venture row (slug, role,
// created_at) and /api/tenants/:slug/settings for the live settings card
// (display_name, cadence, dues, currency, timezone, current_period). The
// remaining fields below — members, totals, period rhythm — still mock at
// the load-edge until their endpoints ship.
//
// TODO(api): GET /api/ventures/:slug/members  → Vec<MemberRow>
// TODO(api): GET /api/ventures/:slug/rhythm   → last N PeriodCells + totals

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct VentureDetail {
    slug: String,
    display_name: String,
    role: String,
    purpose: Option<String>,
    cadence: String,  // "weekly" | "monthly" | "yearly"
    currency: String, // "MYR" | "USD" | …
    timezone: String, // "Asia/Kuala_Lumpur"
    dues_amount_cents: i64,
    current_period: String, // e.g. "2026-05"
    period_status: String,  // "open" | "closing" | "locked"
    member_count: usize,
    contribution_total_cents: i64,
    outstanding_count: usize,
    created_at: String, // ISO 8601
    last_periods: Vec<PeriodCell>,
    members: Vec<MemberRow>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct PeriodCell {
    label: String,  // "2025-12" | "2026-W17"
    status: String, // "verified" | "partial" | "missed" | "current" | "upcoming"
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct MemberRow {
    email: String,
    display_name: Option<String>,
    role: String,
    joined_at: String,
    last_period_status: String, // "verified" | "submitted" | "missed" | "—"
}

// ─── Loader ────────────────────────────────────────────────────────────────

async fn fetch_venture(slug: String) -> Result<VentureDetail, ApiError> {
    // Confirm the caller actually has access — pulls from the existing
    // /api/me/ventures endpoint and looks for this slug.
    let resp = authed(reqwest::Method::GET, "/api/me/ventures")?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;

    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }

    #[derive(Deserialize)]
    struct Row {
        slug: String,
        role: String,
        created_at: String,
    }
    #[derive(Deserialize)]
    struct VR {
        ventures: Vec<Row>,
    }
    let body: VR = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))?;

    let row = body
        .ventures
        .into_iter()
        .find(|v| v.slug == slug)
        .ok_or(ApiError::Other(
            "venture not found in your memberships".into(),
        ))?;

    // ── Real settings from /api/tenants/:slug/settings ────────────────────
    let settings_resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{}/settings", slug),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;

    if !settings_resp.status().is_success() {
        return Err(into_api_error(settings_resp).await);
    }

    #[derive(Deserialize)]
    struct SettingsView {
        display_name: String,
        timezone: String,
        currency: String,
        cadence: String,
        dues_amount_cents: i64,
        current_period: String,
        #[serde(default)]
        purpose: Option<String>,
    }
    #[derive(Deserialize)]
    struct SettingsBody {
        settings: SettingsView,
    }
    let s: SettingsBody = settings_resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode settings: {e}")))?;
    let s = s.settings;

    // ── Real members from /api/tenants/:slug/members ──────────────────────
    let members_resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{}/members", slug),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;

    if !members_resp.status().is_success() {
        return Err(into_api_error(members_resp).await);
    }

    #[derive(Deserialize)]
    struct MembersBody {
        members: Vec<MemberRow>,
    }
    let mb: MembersBody = members_resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode members: {e}")))?;
    let members = mb.members;
    let member_count = members.len();

    // ── Mock the rhythm/members until those endpoints ship ────────────────
    Ok(VentureDetail {
        slug: row.slug,
        display_name: s.display_name,
        role: row.role,
        purpose: s.purpose,
        cadence: s.cadence,
        currency: s.currency,
        timezone: s.timezone,
        dues_amount_cents: s.dues_amount_cents,
        current_period: s.current_period,
        period_status: "open".into(),
        member_count,
        contribution_total_cents: 0,
        outstanding_count: 0,
        created_at: row.created_at,
        last_periods: vec![
            PeriodCell {
                label: "2025-12".into(),
                status: "verified".into(),
            },
            PeriodCell {
                label: "2026-01".into(),
                status: "verified".into(),
            },
            PeriodCell {
                label: "2026-02".into(),
                status: "partial".into(),
            },
            PeriodCell {
                label: "2026-03".into(),
                status: "verified".into(),
            },
            PeriodCell {
                label: "2026-04".into(),
                status: "verified".into(),
            },
            PeriodCell {
                label: "2026-05".into(),
                status: "current".into(),
            },
        ],
        members,
    })
}

#[derive(Serialize, Default)]
struct UpdateSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cadence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dues_amount_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    purpose: Option<String>,
}

async fn save_settings(slug: &str, req: UpdateSettingsRequest) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::PATCH,
        &format!("/api/tenants/{slug}/settings"),
    )?
    .json(&req)
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    Ok(())
}

// ─── My-period contributions API ───────────────────────────────────────────

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct PeriodSummary {
    period: String,
    cadence: String,
    currency: String,
    dues_cents: i64,
    paid_cents: i64,
    remaining_cents: i64,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct Contribution {
    id: String,
    amount_cents: i64,
    status: String,
    note: Option<String>,
    submitted_at: String,
    #[serde(default)]
    proof_key: Option<String>,
}

#[derive(Deserialize)]
struct MyContribsBody {
    summary: PeriodSummary,
    contributions: Vec<Contribution>,
}

async fn fetch_my_contributions(
    slug: String,
) -> Result<(PeriodSummary, Vec<Contribution>), ApiError> {
    let resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{slug}/contributions/me"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: MyContribsBody = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode contributions: {e}")))?;
    Ok((body.summary, body.contributions))
}

#[derive(Serialize)]
struct PostContributionRequest {
    amount_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_key: Option<String>,
}

async fn submit_contribution(slug: &str, req: PostContributionRequest) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::POST,
        &format!("/api/tenants/{slug}/contributions"),
    )?
    .json(&req)
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    Ok(())
}

// ─── Proof-of-payment upload ───────────────────────────────────────────────

/// Hard cap shared with the gateway. Keep in sync with `MAX_PROOF_BYTES` in
/// `crate/gateway/src/routes/proofs.rs` so the UI can short-circuit oversized
/// files instead of round-tripping to a 413.
const MAX_PROOF_BYTES: usize = 10 * 1024 * 1024;

#[derive(Deserialize)]
struct UploadProofResponse {
    proof_key: String,
}

#[derive(Deserialize)]
struct ProofUrlResponse {
    url: String,
}

/// Map a filename's extension to one of the four mime types the gateway
/// accepts for proofs. Returns `None` for everything else; the upload would
/// 415 on the server anyway, but bouncing locally is friendlier.
fn proof_content_type(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".webp") {
        Some("image/webp")
    } else if lower.ends_with(".pdf") {
        Some("application/pdf")
    } else {
        None
    }
}

async fn upload_proof(
    slug: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<String, ApiError> {
    let resp = authed(
        reqwest::Method::POST,
        &format!("/api/tenants/{slug}/proofs"),
    )?
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(bytes)
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: UploadProofResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode upload: {e}")))?;
    Ok(body.proof_key)
}

async fn fetch_proof_url(slug: &str, proof_key: &str) -> Result<String, ApiError> {
    let encoded = urlencode(proof_key);
    let resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{slug}/proofs/url?key={encoded}"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: ProofUrlResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode proof url: {e}")))?;
    Ok(body.url)
}

/// Minimal percent-encode for the proof_key query value. Keys are
/// `tenants/{slug}/{frag}/{uuid}.{ext}` — `/` and `.` need encoding when
/// they'd otherwise be path delimiters; we encode aggressively to be safe.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        let c = *b;
        let safe = c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(c as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", c));
        }
    }
    out
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn fmt_money(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs() as i64;
    let major = abs / 100;
    let minor = abs % 100;
    // Group thousands with a thin separator-friendly comma.
    let s = major.to_string();
    let mut grouped = String::new();
    let chars: Vec<char> = s.chars().rev().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*c);
    }
    let major_str: String = grouped.chars().rev().collect();
    format!("{sign}{major_str}.{minor:02} {currency}")
}

fn cadence_rhythm(cadence: &str) -> &'static str {
    match cadence {
        "weekly" => "Each ISO week",
        "yearly" => "Each calendar year",
        _ => "Each calendar month",
    }
}

fn role_pill(role: &str) -> &'static str {
    match role {
        "owner" => "pill pill-evergreen",
        "treasurer" => "pill pill-amber",
        _ => "pill pill-neutral",
    }
}

fn member_status_pill(s: &str) -> &'static str {
    match s {
        "verified" => "pill pill-positive",
        "submitted" => "pill pill-neutral",
        "missed" => "pill pill-negative",
        _ => "pill pill-neutral",
    }
}

fn period_cell_class(status: &str) -> &'static str {
    match status {
        "verified" => "h-8 w-full rounded-sm bg-positive/85 border border-positive/40",
        "partial"  => "h-8 w-full rounded-sm bg-amber/70 border border-amber/40",
        "missed"   => "h-8 w-full rounded-sm bg-negative/75 border border-negative/40",
        "current"  => "h-8 w-full rounded-sm bg-evergreen border border-evergreen-deep ring-1 ring-evergreen/30",
        _          => "h-8 w-full rounded-sm bg-bone-soft border border-rule",
    }
}

// ─── Overview Page ────────────────────────────────────────────────────────

#[component]
pub fn OverviewPage(slug: String) -> Element {
    let slug_for_load = slug.clone();
    let venture = use_resource(move || {
        let s = slug_for_load.clone();
        async move { fetch_venture(s).await }
    });

    rsx! {
        Sidenav { active: "venture-overview".to_string(),
            {match venture() {
                None => rsx! { LoadingShell { slug: slug.clone() } },
                Some(Err(ApiError::NotSignedIn)) | Some(Err(ApiError::Unauthorized)) => {
                    rsx! { SignInPrompt {} }
                }
                Some(Err(e)) => {
                    let msg = e.to_string();
                    rsx! { ErrorPanel { message: msg } }
                }
                Some(Ok(v)) => rsx! {
                    OverviewBody { v: v }
                },
            }}
        }
    }
}

// ─── Contribute Page ──────────────────────────────────────────────────────

#[component]
pub fn ContributePage(slug: String) -> Element {
    let slug_for_load = slug.clone();
    let venture = use_resource(move || {
        let s = slug_for_load.clone();
        async move { fetch_venture(s).await }
    });

    rsx! {
        Sidenav { active: "venture-contribute".to_string(),
            {match venture() {
                None => rsx! { LoadingShell { slug: slug.clone() } },
                Some(Err(ApiError::NotSignedIn)) | Some(Err(ApiError::Unauthorized)) => {
                    rsx! { SignInPrompt {} }
                }
                Some(Err(e)) => {
                    let msg = e.to_string();
                    rsx! { ErrorPanel { message: msg } }
                }
                Some(Ok(v)) => rsx! {
                    ContributeBody { v: v }
                },
            }}
        }
    }
}

// ─── Manage Page (admin only) ─────────────────────────────────────────────

#[component]
pub fn ManagePage(slug: String) -> Element {
    let slug_for_load = slug.clone();
    let mut venture = use_resource(move || {
        let s = slug_for_load.clone();
        async move { fetch_venture(s).await }
    });

    rsx! {
        Sidenav { active: "venture-manage".to_string(),
            {match venture() {
                None => rsx! { LoadingShell { slug: slug.clone() } },
                Some(Err(ApiError::NotSignedIn)) | Some(Err(ApiError::Unauthorized)) => {
                    rsx! { SignInPrompt {} }
                }
                Some(Err(e)) => {
                    let msg = e.to_string();
                    rsx! { ErrorPanel { message: msg } }
                }
                Some(Ok(v)) => {
                    let is_admin = v.role == "owner" || v.role == "treasurer";
                    if !is_admin {
                        rsx! { NotAdminPanel { slug: v.slug.clone() } }
                    } else {
                        rsx! {
                            ManageBody {
                                v: v,
                                on_saved: move |_| venture.restart(),
                            }
                        }
                    }
                }
            }}
        }
    }
}

#[component]
fn LoadingShell(slug: String) -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            p { class: "eyebrow mb-3", "VENTURE · {slug}" }
            div {
                class: "card p-10 text-center text-ink-soft text-[14px]",
                "Opening the ledger…"
            }
        }
    }
}

#[component]
fn SignInPrompt() -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div {
                class: "card p-8",
                p { class: "eyebrow mb-2", "SESSION EXPIRED" }
                h2 { class: "font-display text-[20px] font-semibold text-ink", "You're not signed in" }
                p { class: "mt-2 text-[14px] text-ink-soft",
                    "Your sign-in session expired. Sign in again to view this venture."
                }
                a {
                    href: "/login",
                    class: "mt-5 inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper text-[14px] font-medium px-5 py-2.5 rounded-md transition-colors",
                    "Sign in"
                    span { class: "text-[16px] leading-none", "→" }
                }
            }
        }
    }
}

#[component]
fn ErrorPanel(message: String) -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div {
                class: "card p-8",
                p { class: "eyebrow !text-negative mb-2", "ERROR" }
                p { class: "text-[14px] text-ink-soft", "{message}" }
                a { href: "/dashboard",
                    class: "mt-5 inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40",
                    "← Back to dashboard"
                }
            }
        }
    }
}

#[component]
fn OverviewBody(v: VentureDetail) -> Element {
    let mut drawer_open = use_signal(|| false);

    let pill_cls = role_pill(&v.role);
    let is_admin = v.role == "owner" || v.role == "treasurer";
    let cadence_word = cadence_rhythm(&v.cadence);
    let dues_label = fmt_money(v.dues_amount_cents, &v.currency);

    let period_status_pill = match v.period_status.as_str() {
        "open" => "pill pill-evergreen",
        "closing" => "pill pill-amber",
        _ => "pill pill-neutral",
    };

    rsx! {
        // ── Breadcrumb strip ─────────────────────────────────────────────
        div {
            class: "px-4 sm:px-6 lg:px-12 pt-7 pb-3 flex items-center gap-3 text-[12px] text-ink-faint font-mono tracking-[0.12em] uppercase rise",
            span { class: "text-ink-soft", "ns={v.slug}" }
        }

        // ── Hero ────────────────────────────────────────────────────────
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-2 pb-10 max-w-[1140px] rise",
            style: "animation-delay: 0.04s",

            div {
                class: "flex flex-col gap-6 lg:flex-row lg:items-start lg:justify-between lg:gap-8",

                div { class: "min-w-0 flex-1",
                    p { class: "eyebrow mb-4",
                        "VENTURE LEDGER · "
                        span { class: "text-evergreen", "{v.slug}" }
                    }
                    h1 {
                        class: "display text-[clamp(2rem,5vw,3.5rem)] font-light leading-[1.04] text-ink break-words",
                        "{v.display_name}"
                    }
                    div {
                        class: "mt-5 flex items-center gap-2 sm:gap-3 flex-wrap",
                        span { class: "{pill_cls}", "{v.role}" }
                        span { class: "{period_status_pill}", "Period {v.current_period} · {v.period_status}" }
                        span { class: "text-[12.5px] text-ink-faint font-mono tracking-[0.06em]",
                            "{cadence_word} · dues {dues_label}"
                        }
                    }
                }

                // Primary action — open the Contribute drawer.
                // On mobile/tablet we stack action buttons full-width below the
                // heading; on lg+ they sit to the right.
                div { class: "flex items-stretch sm:items-center gap-2 sm:gap-3 flex-wrap lg:shrink-0",
                    button {
                        r#type: "button",
                        class: "flex-1 sm:flex-none inline-flex items-center justify-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper text-[14px] font-medium px-5 py-3 rounded-md transition-colors shadow-[0_8px_24px_-12px_rgba(31,77,61,0.45)]",
                        onclick: move |_| drawer_open.set(true),
                        span { class: "text-[16px] leading-none", "+" }
                        "Contribute"
                    }
                    if is_admin {
                        a {
                            href: "/ventures/{v.slug}/manage",
                            class: "flex-1 sm:flex-none inline-flex items-center justify-center gap-2 border border-rule bg-paper hover:bg-bone-soft text-ink-soft hover:text-ink text-[13px] font-medium px-4 py-2.5 rounded-md transition-colors",
                            span { class: "text-evergreen", "⚙" }
                            "Manage"
                        }
                    }
                }
            }
        }

        // ── Holding summary — the venture's balance sheet headline ─────
        HoldingSummary {
            slug: v.slug.clone(),
            currency: v.currency.clone(),
            is_admin: is_admin,
        }

        // ── Purpose pull-quote ───────────────────────────────────────────
        PurposeBlock { purpose: v.purpose.clone() }

        // ── Accumulation chart over the venture's lifetime ─────────────
        AccumulationChart { slug: v.slug.clone(), currency: v.currency.clone() }

        // ── Rhythm strip — last six periods ─────────────────────────────
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-12 max-w-[1140px] rise",
            style: "animation-delay: 0.20s",

            div {
                class: "flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4 mb-5",
                p { class: "eyebrow", "RHYTHM · LAST SIX PERIODS" }
                p { class: "text-[12px] text-ink-faint font-mono",
                    "{cadence_word}"
                }
            }

            div {
                class: "grid grid-cols-3 sm:grid-cols-6 gap-2",
                for cell in v.last_periods.iter() {
                    div {
                        class: "flex flex-col gap-2",
                        div { class: "{period_cell_class(&cell.status)}" }
                        p {
                            class: "font-mono text-[10.5px] tracking-[0.04em] text-ink-faint text-center tnum",
                            "{cell.label}"
                        }
                    }
                }
            }

            div {
                class: "mt-5 flex flex-wrap gap-x-5 gap-y-2 text-[11px] text-ink-faint font-mono uppercase tracking-[0.1em]",
                LegendDot { tone: "verified" }
                LegendDot { tone: "partial" }
                LegendDot { tone: "missed" }
                LegendDot { tone: "current" }
            }
        }

        // ── Audit log — every payment recorded into the selected period ─
        AuditLogPanel { slug: v.slug.clone() }

        // ── Drawer (overlay) ────────────────────────────────────────────
        if drawer_open() {
            ContributeDrawer {
                slug: v.slug.clone(),
                display_name: v.display_name.clone(),
                currency: v.currency.clone(),
                period: v.current_period.clone(),
                on_close: move |_| drawer_open.set(false),
            }
        }
    }
}

// ─── Contribute body (full-page version, route /contribute) ───────────────

#[component]
fn ContributeBody(v: VentureDetail) -> Element {
    let cadence_word = cadence_rhythm(&v.cadence);
    let dues_label = fmt_money(v.dues_amount_cents, &v.currency);

    rsx! {
        // Breadcrumb
        div {
            class: "px-4 sm:px-6 lg:px-12 pt-7 pb-3 flex items-center gap-3 text-[12px] text-ink-faint font-mono tracking-[0.12em] uppercase rise",
            a { href: "/ventures/{v.slug}", class: "hover:text-evergreen transition-colors", "ns={v.slug}" }
            span { "›" }
            span { class: "text-ink-soft", "Contribute" }
        }

        // Hero — quiet, focused on the act of paying.
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-2 pb-8 max-w-[1140px] rise",
            style: "animation-delay: 0.04s",

            p { class: "eyebrow mb-3",
                "CONTRIBUTE · "
                span { class: "text-evergreen", "{v.slug}" }
            }
            h1 {
                class: "display text-[clamp(1.75rem,4vw,2.75rem)] font-light leading-[1.08] text-ink",
                "Record a payment to {v.display_name}"
            }
            p {
                class: "mt-4 text-[14px] text-ink-soft max-w-2xl",
                "{cadence_word} · dues "
                span { class: "text-ink", "{dues_label}" }
                ". Partial payments add up — submit any amount up to the cycle cap. Period {v.current_period} is open."
            }
        }

        // The form + history live in MyPeriodPanel — same component the drawer uses.
        MyPeriodPanel { slug: v.slug.clone(), currency: v.currency.clone() }

        // Footer link — back to overview to see collective progress.
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-16 max-w-[1140px]",
            div { class: "border-t border-rule pt-6 flex flex-wrap items-center justify-between gap-4",
                p { class: "text-[12.5px] text-ink-faint font-mono",
                    "Looking for the venture-wide collected total?"
                }
                a {
                    href: "/ventures/{v.slug}",
                    class: "inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40 hover:border-evergreen transition-colors",
                    "← Back to overview"
                }
            }
        }
    }
}

// ─── Manage body (admin only) ─────────────────────────────────────────────

#[component]
fn ManageBody(v: VentureDetail, on_saved: EventHandler<()>) -> Element {
    let name = use_signal(|| v.display_name.clone());
    let cad = use_signal(|| v.cadence.clone());
    let dues_major = use_signal(|| (v.dues_amount_cents / 100).to_string());
    let tz = use_signal(|| v.timezone.clone());
    let cur = use_signal(|| v.currency.clone());
    let purpose = use_signal(|| v.purpose.clone().unwrap_or_default());
    let mut saving = use_signal(|| false);
    let mut save_error: Signal<Option<String>> = use_signal(|| None);
    let mut save_flash: Signal<Option<String>> = use_signal(|| None);

    let pill_cls = role_pill(&v.role);
    let cadence_word = cadence_rhythm(&v.cadence);
    let created = v.created_at.get(..10).unwrap_or("").to_string();
    let slug_for_save = v.slug.clone();

    rsx! {
        // Breadcrumb
        div {
            class: "px-4 sm:px-6 lg:px-12 pt-7 pb-3 flex items-center gap-3 text-[12px] text-ink-faint font-mono tracking-[0.12em] uppercase rise",
            a { href: "/ventures/{v.slug}", class: "hover:text-evergreen transition-colors", "ns={v.slug}" }
            span { "›" }
            span { class: "text-ink-soft", "Manage" }
        }

        // Hero — admin tone.
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-2 pb-8 max-w-[1140px] rise",
            style: "animation-delay: 0.04s",

            div { class: "flex flex-wrap items-end justify-between gap-6",
                div {
                    p { class: "eyebrow mb-3",
                        "MANAGE · "
                        span { class: "text-evergreen", "{v.slug}" }
                    }
                    h1 {
                        class: "display text-[clamp(1.75rem,4vw,2.75rem)] font-light leading-[1.08] text-ink",
                        "{v.display_name}"
                    }
                    div {
                        class: "mt-4 flex items-center gap-3 flex-wrap",
                        span { class: "{pill_cls}", "{v.role}" }
                        span { class: "text-[12.5px] text-ink-faint font-mono tracking-[0.06em]",
                            "Opened {created} · {cadence_word}"
                        }
                    }
                }
                div {
                    class: "shrink-0 inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep disabled:opacity-60 disabled:cursor-not-allowed text-paper text-[13px] font-medium rounded-md transition-colors",
                    style: "padding: 0;",
                    button {
                        r#type: "button",
                        class: "px-5 py-3 disabled:cursor-not-allowed",
                        disabled: saving(),
                        onclick: {
                            let slug = slug_for_save.clone();
                            let name = name;
                            let cad = cad;
                            let dues_major = dues_major;
                            let tz = tz;
                            let cur = cur;
                            let purpose = purpose;
                            move |_| {
                                let slug = slug.clone();
                                async move {
                                    saving.set(true);
                                    save_error.set(None);
                                    save_flash.set(None);
                                    let dues_cents = dues_major()
                                        .trim()
                                        .parse::<i64>()
                                        .unwrap_or(0)
                                        .saturating_mul(100);
                                    let req = UpdateSettingsRequest {
                                        display_name: Some(name()),
                                        timezone: Some(tz()),
                                        currency: Some(cur()),
                                        cadence: Some(cad()),
                                        dues_amount_cents: Some(dues_cents),
                                        purpose: Some(purpose()),
                                    };
                                    match save_settings(&slug, req).await {
                                        Ok(()) => {
                                            save_flash.set(Some("Saved.".into()));
                                            on_saved.call(());
                                        }
                                        Err(e) => save_error.set(Some(e.to_string())),
                                    }
                                    saving.set(false);
                                }
                            }
                        },
                        if saving() { "Saving…" } else { "Save changes" }
                    }
                }
            }

            if let Some(msg) = save_error() {
                p {
                    class: "mt-4 text-[12.5px] text-negative font-mono",
                    "Save failed: {msg}"
                }
            }
            if let Some(msg) = save_flash() {
                p {
                    class: "mt-4 text-[12.5px] text-positive font-mono",
                    "{msg}"
                }
            }
        }

        // Editor: purpose
        EditPurposeBlock { text: purpose }

        // Editor: cadence + dues + tz + currency
        EditCadenceBlock {
            name: name,
            cad: cad,
            dues_major: dues_major,
            tz: tz,
            cur: cur,
        }

        // Members table — full
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-16 max-w-[1140px] rise",
            style: "animation-delay: 0.20s",

            div {
                class: "flex flex-col gap-3 sm:flex-row sm:items-baseline sm:justify-between sm:gap-6 mb-5",
                div {
                    p { class: "eyebrow", "MEMBERS · {v.member_count}" }
                    h2 { class: "mt-2 font-display text-[22px] sm:text-[24px] font-semibold text-ink leading-tight",
                        "Who shows up to this ledger"
                    }
                }
                if v.role == "owner" {
                    a {
                        href: "/ventures/{v.slug}/invites",
                        class: "self-start sm:self-auto inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40 hover:border-evergreen transition-colors",
                        "+ Invite member"
                    }
                }
            }

            div {
                class: "card overflow-hidden",
                div {
                    class: "hidden sm:grid grid-cols-[2.4fr_1fr_1fr_1fr] gap-4 px-5 py-3 bg-bone-soft border-b border-rule text-[11px] text-ink-faint font-mono uppercase tracking-[0.14em]",
                    span { "Member" }
                    span { "Role" }
                    span { "Joined" }
                    span { class: "text-right", "Period {v.current_period}" }
                }
                if v.members.is_empty() {
                    div {
                        class: "px-5 py-10 text-center",
                        p { class: "eyebrow mb-2", "NO MEMBERS YET" }
                        p { class: "text-[13px] text-ink-soft",
                            if v.role == "owner" {
                                "Invite people to join this venture."
                            } else {
                                "Once members accept their invites, they'll appear here."
                            }
                        }
                    }
                } else {
                    for (i, m) in v.members.iter().enumerate() {
                        MemberRowView { idx: i, member: m.clone() }
                    }
                }
            }

            p {
                class: "mt-6 text-[12px] text-ink-faint font-mono leading-[1.6] max-w-2xl",
                "Members listed here are joined via accepted invites in the control plane. "
                "Removal does not delete prior contributions — period-locked rows remain visible "
                "in the audit log for the lifetime of the venture."
            }
        }
    }
}

// ─── Not-admin gate for /manage ───────────────────────────────────────────

#[component]
fn NotAdminPanel(slug: String) -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div { class: "card p-8",
                p { class: "eyebrow !text-amber mb-2", "ADMIN ONLY" }
                h2 { class: "font-display text-[22px] font-semibold text-ink",
                    "Manage is for owners and treasurers"
                }
                p { class: "mt-2 text-[14px] text-ink-soft max-w-2xl",
                    "You're a member of this venture, but settings and member management are reserved \
                    for admins. You can still see the venture's pool, lifetime totals, and submit \
                    contributions from the overview."
                }
                div { class: "mt-6 flex flex-wrap items-center gap-4",
                    a {
                        href: "/ventures/{slug}",
                        class: "inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper text-[13px] font-medium px-4 py-2.5 rounded-md transition-colors",
                        "Open overview →"
                    }
                    a {
                        href: "/ventures/{slug}/contribute",
                        class: "inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40 hover:border-evergreen transition-colors",
                        "Or contribute now"
                    }
                }
            }
        }
    }
}

// ─── Contribute drawer (overlay launched from Overview) ───────────────────

#[component]
fn ContributeDrawer(
    slug: String,
    display_name: String,
    currency: String,
    period: String,
    on_close: EventHandler<()>,
) -> Element {
    let slug_full_link = slug.clone();
    rsx! {
        // Overlay scrim — click closes.
        div {
            class: "drawer-overlay",
            onclick: move |_| on_close.call(()),
        }
        // Right-side panel.
        aside {
            class: "drawer",
            // Header
            div {
                class: "px-6 pt-6 pb-4 border-b border-rule flex items-start justify-between gap-4",
                div {
                    p { class: "eyebrow mb-1", "CONTRIBUTE · PERIOD {period}" }
                    h2 {
                        class: "font-display text-[22px] font-semibold text-ink leading-tight",
                        "{display_name}"
                    }
                }
                button {
                    r#type: "button",
                    onclick: move |_| on_close.call(()),
                    class: "shrink-0 h-9 w-9 rounded-md text-ink-soft hover:bg-bone-soft hover:text-ink transition-colors flex items-center justify-center text-[18px] leading-none",
                    "×"
                }
            }

            // Body — same MyPeriodPanel the full page uses, but in compact
            // (vertically stacked) mode for the narrow drawer.
            div {
                class: "drawer-body",
                MyPeriodPanel {
                    slug: slug.clone(),
                    currency: currency.clone(),
                    compact: true,
                }
            }

            // Footer — escape hatch to full page.
            div {
                class: "px-6 py-4 border-t border-rule bg-bone-soft/60 flex items-center justify-between text-[12.5px]",
                p { class: "text-ink-faint font-mono", "Need more room?" }
                a {
                    href: "/ventures/{slug_full_link}/contribute",
                    class: "text-evergreen hover:text-evergreen-deep border-b border-evergreen/40 hover:border-evergreen",
                    "Open full page →"
                }
            }
        }
    }
}

// ─── Sub-components ────────────────────────────────────────────────────────

#[component]
fn PurposeBlock(purpose: Option<String>) -> Element {
    let body = purpose.unwrap_or_else(|| {
        "No purpose has been written for this venture yet. Open the editor to record \
         what this ledger exists to do — it appears here, on every member's view."
            .into()
    });
    rsx! {
        section {
            class: "relative px-4 sm:px-6 lg:px-12 py-10 max-w-[1140px] border-t border-rule rise",
            style: "animation-delay: 0.08s",

            div { class: "grid grid-cols-1 lg:grid-cols-12 gap-8 items-start",
                // Left rail label
                div { class: "lg:col-span-3",
                    p { class: "eyebrow", "PURPOSE" }
                    p { class: "mt-3 text-[12.5px] text-ink-faint leading-[1.55] font-light",
                        "Why this pool exists. Read at every contribution."
                    }
                }

                // Pull-quote
                blockquote {
                    class: "lg:col-span-9 relative pl-8 border-l-2 border-copper/60",
                    span {
                        class: "absolute -left-1 -top-3 font-display text-[64px] leading-none text-copper/70",
                        style: "font-variation-settings: 'opsz' 144, 'SOFT' 100;",
                        "“"
                    }
                    p {
                        class: "font-display italic text-[clamp(1.25rem,2.4vw,1.75rem)] leading-[1.45] text-ink font-light",
                        style: "font-variation-settings: 'opsz' 96, 'SOFT' 80;",
                        "{body}"
                    }
                }
            }
        }
    }
}

#[component]
fn EditPurposeBlock(mut text: Signal<String>) -> Element {
    let count = text.read().chars().count();
    let count_label = format!("{count} / 600");
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 py-10 max-w-[1140px] border-t border-rule",

            div { class: "grid grid-cols-1 lg:grid-cols-12 gap-8 items-start",
                div { class: "lg:col-span-3",
                    p { class: "eyebrow", "PURPOSE" }
                    p { class: "mt-3 text-[12.5px] text-ink-faint leading-[1.55] font-light",
                        "Read at every contribution. Keep it concrete — what this pool funds, who it serves."
                    }
                }
                div { class: "lg:col-span-9",
                    textarea {
                        rows: "5",
                        maxlength: "600",
                        class: "w-full font-display italic text-[20px] leading-[1.5] text-ink bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md p-5 resize-y transition",
                        style: "font-variation-settings: 'opsz' 96, 'SOFT' 80;",
                        value: "{text}",
                        oninput: move |e| text.set(e.value()),
                    }
                    p {
                        class: "mt-2 text-right text-[11px] text-ink-faint font-mono tracking-[0.1em] tnum",
                        "{count_label}"
                    }
                }
            }
        }
    }
}

#[component]
fn EditCadenceBlock(
    mut name: Signal<String>,
    mut cad: Signal<String>,
    mut dues_major: Signal<String>,
    mut tz: Signal<String>,
    mut cur: Signal<String>,
) -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-12 max-w-[1140px] grid grid-cols-1 lg:grid-cols-12 gap-6",

            div { class: "lg:col-span-12 card p-6",
                p { class: "eyebrow mb-5", "EDIT VENTURE" }

                // Display name
                Field { label: "Display name".to_string(), hint: "Shown across the app and on every member's dashboard.".to_string(),
                    input {
                        class: "w-full bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md px-3.5 py-2.5 text-[15px] text-ink transition",
                        value: "{name}",
                        oninput: move |e| name.set(e.value()),
                    }
                }

                Hairline {}

                // Cadence segmented control
                Field { label: "Cadence".to_string(), hint: "How often dues are owed. Changing cadence affects only future periods.".to_string(),
                    div {
                        class: "inline-flex rounded-md border border-rule bg-bone-soft p-1 gap-1",
                        for option in ["weekly", "monthly", "yearly"].iter() {
                            {
                                let opt = option.to_string();
                                let active = cad() == opt;
                                let cls = if active {
                                    "px-4 py-2 rounded-[6px] text-[13px] font-medium bg-paper text-evergreen shadow-[0_1px_0_rgba(26,31,44,0.04)] border border-rule"
                                } else {
                                    "px-4 py-2 rounded-[6px] text-[13px] text-ink-soft hover:text-ink transition-colors"
                                };
                                rsx! {
                                    button {
                                        class: "{cls}",
                                        onclick: move |_| cad.set(opt.clone()),
                                        "{option}"
                                    }
                                }
                            }
                        }
                    }
                }

                Hairline {}

                // Dues + currency — paired in a single label-rail row.
                // Each Field's internal label rail would compound here, so we
                // render one label/hint on the left and place the two inputs
                // side-by-side on the right.
                div { class: "grid grid-cols-1 lg:grid-cols-[260px_1fr] gap-3 lg:gap-8 lg:items-start py-4",
                    div {
                        p { class: "eyebrow mb-1", "Dues per cycle" }
                        p { class: "text-[12px] text-ink-faint leading-[1.55]",
                            "Whole units of the venture's currency. One currency per venture."
                        }
                    }
                    div { class: "grid grid-cols-[1fr_140px] gap-3",
                        div { class: "flex items-stretch border border-rule rounded-md overflow-hidden focus-within:border-evergreen focus-within:ring-2 focus-within:ring-evergreen/15 transition",
                            span {
                                class: "px-3 flex items-center bg-bone-soft text-ink-soft font-mono text-[13px] border-r border-rule tnum",
                                "{cur}"
                            }
                            input {
                                r#type: "number",
                                min: "0",
                                step: "1",
                                class: "flex-1 min-w-0 px-3.5 py-2.5 text-[15px] text-ink bg-paper outline-none tnum",
                                value: "{dues_major}",
                                oninput: move |e| dues_major.set(e.value()),
                            }
                        }
                        select {
                            class: "bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md px-3 py-2.5 text-[15px] text-ink transition appearance-none font-mono tnum",
                            value: "{cur}",
                            onchange: move |e| cur.set(e.value()),
                            for code in ["MYR", "USD", "EUR", "SGD", "GBP", "INR", "AUD"].iter() {
                                option { value: "{code}", "{code}" }
                            }
                        }
                    }
                }

                Hairline {}

                Field { label: "Time zone".to_string(), hint: "Used to lock periods at the venture's local midnight.".to_string(),
                    select {
                        class: "w-full bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md px-3 py-2.5 text-[15px] text-ink transition appearance-none",
                        value: "{tz}",
                        onchange: move |e| tz.set(e.value()),
                        for zone in [
                            "UTC",
                            "Asia/Kuala_Lumpur",
                            "Asia/Singapore",
                            "Asia/Kolkata",
                            "Europe/London",
                            "America/New_York",
                            "America/Los_Angeles",
                            "Australia/Sydney",
                        ].iter() {
                            option { value: "{zone}", "{zone}" }
                        }
                    }
                }

                p {
                    class: "mt-6 text-[12px] text-ink-faint font-mono leading-[1.6]",
                    "Period locks already in place are not retroactively unsealed by edits here. "
                    "Past contributions remain bound to the cadence under which they were submitted."
                }
            }
        }
    }
}

#[component]
fn FactRow(label: String, value: String) -> Element {
    rsx! {
        div {
            p { class: "eyebrow mb-1", "{label}" }
            p { class: "font-display text-[20px] text-ink font-light tnum tracking-[-0.005em]",
                "{value}"
            }
        }
    }
}

#[component]
fn Metric(label: String, value: String, suffix: String) -> Element {
    rsx! {
        div {
            p { class: "eyebrow mb-2", "{label}" }
            p {
                class: "font-display text-[clamp(1.75rem,3.5vw,2.25rem)] text-ink font-light leading-[1.05] tnum",
                "{value}"
            }
            p {
                class: "mt-1 text-[12px] text-ink-faint font-mono tracking-[0.06em]",
                "{suffix}"
            }
        }
    }
}

#[component]
fn LegendDot(tone: String) -> Element {
    let (cls, label) = match tone.as_str() {
        "verified" => (
            "h-2.5 w-2.5 rounded-full bg-positive/85 border border-positive/40",
            "Verified",
        ),
        "partial" => (
            "h-2.5 w-2.5 rounded-full bg-amber/70 border border-amber/40",
            "Partial",
        ),
        "missed" => (
            "h-2.5 w-2.5 rounded-full bg-negative/75 border border-negative/40",
            "Missed",
        ),
        "current" => (
            "h-2.5 w-2.5 rounded-full bg-evergreen border border-evergreen-deep",
            "Current",
        ),
        _ => (
            "h-2.5 w-2.5 rounded-full bg-bone-soft border border-rule",
            "—",
        ),
    };
    rsx! {
        span {
            class: "inline-flex items-center gap-2",
            span { class: "{cls}" }
            span { "{label}" }
        }
    }
}

#[component]
fn MemberRowView(idx: usize, member: MemberRow) -> Element {
    let initial = member
        .display_name
        .clone()
        .and_then(|n| n.chars().next().map(|c| c.to_string()))
        .or_else(|| {
            member
                .email
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
        })
        .unwrap_or_else(|| "?".into());
    let stripe = if idx % 2 == 1 {
        "bg-bone-soft/50"
    } else {
        "bg-paper"
    };
    let role_cls = role_pill(&member.role);
    let status_cls = member_status_pill(&member.last_period_status);
    let display = member
        .display_name
        .clone()
        .unwrap_or_else(|| member.email.clone());

    rsx! {
        div {
            class: "flex flex-col gap-2 sm:gap-4 sm:grid sm:grid-cols-[2.4fr_1fr_1fr_1fr] sm:items-center px-4 sm:px-5 py-3.5 border-b border-rule-soft last:border-b-0 {stripe} hover:bg-evergreen-soft/40 transition-colors",
            // Member identity
            div { class: "flex items-center gap-3 min-w-0",
                span {
                    class: "h-8 w-8 shrink-0 rounded-full bg-evergreen/10 text-evergreen flex items-center justify-center font-display text-[14px] font-semibold",
                    "{initial}"
                }
                div { class: "min-w-0",
                    p { class: "text-[14px] text-ink font-medium truncate", "{display}" }
                    p { class: "text-[12px] text-ink-faint font-mono truncate", "{member.email}" }
                }
            }
            // Mobile: meta row (role · joined · status) sits below identity.
            // Desktop: each cell goes in its own grid column.
            div { class: "flex items-center gap-2 flex-wrap sm:contents",
                span { class: "{role_cls}", "{member.role}" }
                p { class: "text-[12px] text-ink-soft font-mono tnum sm:text-[12.5px]",
                    "{member.joined_at.get(..10).unwrap_or(&member.joined_at)}"
                }
                span {
                    class: "{status_cls} sm:ml-auto",
                    "{member.last_period_status}"
                }
            }
        }
    }
}

#[component]
fn Field(label: String, hint: String, children: Element) -> Element {
    rsx! {
        div { class: "grid grid-cols-1 lg:grid-cols-[260px_1fr] gap-3 lg:gap-8 lg:items-start py-4",
            div {
                p { class: "eyebrow mb-1", "{label}" }
                p { class: "text-[12px] text-ink-faint leading-[1.55]", "{hint}" }
            }
            div { {children} }
        }
    }
}

#[component]
fn Hairline() -> Element {
    rsx! { div { class: "h-px bg-rule-soft my-2" } }
}

// ─── My-period contribution panel ──────────────────────────────────────────

#[component]
fn MyPeriodPanel(
    slug: String,
    currency: String,
    /// Compact mode for the right drawer — drops the 2-column inner layout
    /// and stacks progress / stats / form vertically. The page version
    /// (`/ventures/:slug/contribute`) leaves this off so the wide layout
    /// breathes across the card.
    #[props(default = false)]
    compact: bool,
) -> Element {
    let slug_for_load = slug.clone();
    let mut data = use_resource(move || {
        let s = slug_for_load.clone();
        async move { fetch_my_contributions(s).await }
    });

    let mut amount_input = use_signal(String::new);
    let mut note_input = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut submit_error: Signal<Option<String>> = use_signal(|| None);
    // Proof-of-payment is split into two signals so re-renders don't clone
    // the (potentially multi-MB) byte buffer on every form keystroke. The
    // meta signal is read by the render path; the bytes signal is only
    // touched at submit time (taken via `.write().take()`).
    let mut selected_proof_meta: Signal<Option<(String, &'static str, usize)>> =
        use_signal(|| None);
    let mut selected_proof_bytes: Signal<Option<Vec<u8>>> = use_signal(|| None);

    let section_class = if compact {
        "rise"
    } else {
        "px-4 sm:px-6 lg:px-12 pb-12 max-w-[1140px] rise"
    };

    rsx! {
        section {
            class: "{section_class}",
            style: "animation-delay: 0.14s",

            {match data() {
                None => rsx! {
                    div {
                        class: "card p-6 text-[14px] text-ink-soft",
                        "Loading your period…"
                    }
                },
                Some(Err(e)) => {
                    let msg = e.to_string();
                    rsx! {
                        div {
                            class: "card p-6 text-[14px] text-negative font-mono",
                            "Couldn't load this period: {msg}"
                        }
                    }
                }
                Some(Ok((summary, rows))) => {
                    // Local copies so the closures own everything they need.
                    let dues   = summary.dues_cents;
                    let paid   = summary.paid_cents;
                    let remain = summary.remaining_cents;
                    let pct = if dues > 0 { (paid * 100 / dues).clamp(0, 100) } else { 100 };
                    let dues_label   = fmt_money(dues, &currency);
                    let paid_label   = fmt_money(paid, &currency);
                    let remain_label = fmt_money(remain, &currency);
                    let period       = summary.period.clone();
                    let bar_cls = if remain == 0 {
                        "h-3 rounded-full bg-positive/85"
                    } else if paid > 0 {
                        "h-3 rounded-full bg-amber/80"
                    } else {
                        "h-3 rounded-full bg-bone-soft border border-rule"
                    };

                    let slug_for_submit = slug.clone();
                    let on_submit = move |_| {
                        let slug = slug_for_submit.clone();
                        async move {
                            submit_error.set(None);
                            // amount in major units → cents
                            let raw = amount_input().trim().to_string();
                            let major: f64 = match raw.parse() {
                                Ok(n) if n > 0.0 => n,
                                _ => {
                                    submit_error.set(Some(
                                        "Enter an amount greater than zero.".into(),
                                    ));
                                    return;
                                }
                            };
                            let cents = (major * 100.0).round() as i64;
                            if cents <= 0 {
                                submit_error.set(Some("Amount rounds to zero.".into()));
                                return;
                            }
                            let note = {
                                let n = note_input().trim().to_string();
                                if n.is_empty() { None } else { Some(n) }
                            };
                            submitting.set(true);

                            // Upload the proof first when one is staged.
                            // Orphaned uploads are invisible to the user
                            // and need a separate sweeper, so we abort
                            // before touching the ledger if the upload
                            // fails. Take ownership of bytes/meta in one
                            // shot; on failure the user re-picks.
                            let staged_bytes = selected_proof_bytes.write().take();
                            let staged_meta = selected_proof_meta.write().take();
                            let proof_key = match (staged_meta, staged_bytes) {
                                (Some((_, ct, _)), Some(bytes)) => {
                                    match upload_proof(&slug, ct, bytes).await {
                                        Ok(k) => Some(k),
                                        Err(e) => {
                                            submit_error.set(Some(format!(
                                                "Proof upload failed: {e}"
                                            )));
                                            submitting.set(false);
                                            return;
                                        }
                                    }
                                }
                                _ => None,
                            };

                            let req = PostContributionRequest {
                                amount_cents: cents,
                                note,
                                proof_key,
                            };
                            match submit_contribution(&slug, req).await {
                                Ok(()) => {
                                    amount_input.set(String::new());
                                    note_input.set(String::new());
                                    data.restart();
                                }
                                Err(e) => submit_error.set(Some(e.to_string())),
                            }
                            submitting.set(false);
                        }
                    };

                    let progress_block = rsx! {
                        div {
                            div {
                                class: "flex items-baseline justify-between",
                                p { class: "eyebrow", "PROGRESS" }
                                p { class: "text-[12px] text-ink-faint font-mono tnum", "{pct}%" }
                            }
                            div {
                                class: "mt-2.5 h-2.5 rounded-full bg-bone-soft border border-rule overflow-hidden",
                                div {
                                    class: "{bar_cls}",
                                    style: "width: {pct}%",
                                }
                            }
                        }
                    };

                    let stats_block = rsx! {
                        div {
                            class: "grid grid-cols-3 gap-3",
                            SmallFact { label: "Dues",      value: dues_label.clone() }
                            SmallFact { label: "Paid",      value: paid_label.clone() }
                            SmallFact { label: "Remaining", value: remain_label.clone() }
                        }
                    };

                    // Bare major-unit string for the Max button — "100.00",
                    // not "100.00 MYR". Drops trailing ".00" cents-only when
                    // the remainder is whole units to keep the input tidy.
                    let max_major = if remain % 100 == 0 {
                        format!("{}", remain / 100)
                    } else {
                        format!("{}.{:02}", remain / 100, remain % 100)
                    };

                    let form_block = rsx! {
                        if remain == 0 && dues > 0 {
                            div {
                                class: "well p-5 flex flex-col items-start gap-2",
                                p { class: "eyebrow !text-positive", "SETTLED" }
                                p { class: "font-display text-[19px] text-ink leading-tight",
                                    "You're paid up for {period}."
                                }
                                p { class: "text-[12.5px] text-ink-soft leading-[1.55]",
                                    "The cap for this cycle has been met. New payments are blocked until the next period opens."
                                }
                            }
                        } else {
                            div { class: "flex flex-col gap-3",
                                p { class: "eyebrow", "RECORD A PAYMENT" }

                                // Amount — full width with currency chip.
                                // The label row hosts a "Max" shortcut that
                                // fills the input with the current remaining
                                // amount, sparing repeat typing of the cap.
                                div {
                                    div {
                                        class: "flex items-baseline justify-between mb-1.5",
                                        label {
                                            class: "text-[11.5px] text-ink-faint font-mono uppercase tracking-[0.1em]",
                                            "Amount"
                                        }
                                        button {
                                            r#type: "button",
                                            disabled: submitting(),
                                            onclick: {
                                                let max = max_major.clone();
                                                move |_| {
                                                    amount_input.set(max.clone());
                                                    submit_error.set(None);
                                                }
                                            },
                                            class: "text-[11px] font-mono uppercase tracking-[0.08em] text-evergreen hover:text-evergreen-deep border border-evergreen/40 hover:border-evergreen bg-evergreen/5 hover:bg-evergreen/10 disabled:opacity-50 disabled:cursor-not-allowed rounded px-2 py-0.5 transition-colors",
                                            "Max"
                                        }
                                    }
                                    div {
                                        class: "flex items-stretch border border-rule rounded-md overflow-hidden focus-within:border-evergreen focus-within:ring-2 focus-within:ring-evergreen/15 transition",
                                        span {
                                            class: "px-3.5 flex items-center bg-bone-soft text-ink-soft font-mono text-[13px] border-r border-rule tnum",
                                            "{currency}"
                                        }
                                        input {
                                            r#type: "number",
                                            min: "0",
                                            step: "0.01",
                                            placeholder: "0.00",
                                            class: "flex-1 min-w-0 px-3.5 py-3 text-[16px] text-ink bg-paper outline-none tnum",
                                            value: "{amount_input}",
                                            oninput: move |e| amount_input.set(e.value()),
                                            disabled: submitting(),
                                        }
                                    }
                                }

                                // Note — full width.
                                div {
                                    label {
                                        class: "block text-[11.5px] text-ink-faint font-mono uppercase tracking-[0.1em] mb-1.5",
                                        "Note "
                                        span { class: "text-ink-faint normal-case font-light", "· optional" }
                                    }
                                    input {
                                        r#type: "text",
                                        placeholder: "e.g. bank transfer ref #1247",
                                        class: "w-full bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md px-3.5 py-3 text-[15px] text-ink transition",
                                        value: "{note_input}",
                                        oninput: move |e| note_input.set(e.value()),
                                        disabled: submitting(),
                                    }
                                }

                                // Proof of payment — optional file input.
                                // The user picks a receipt; we read the bytes
                                // into the `selected_proof` signal here so
                                // submission doesn't have to await the file
                                // engine. Upload itself happens at submit.
                                div {
                                    label {
                                        class: "block text-[11.5px] text-ink-faint font-mono uppercase tracking-[0.1em] mb-1.5",
                                        "Proof of payment "
                                        span { class: "text-ink-faint normal-case font-light", "· optional · jpg/png/webp/pdf, ≤ 10 MB" }
                                    }
                                    input {
                                        r#type: "file",
                                        accept: "image/jpeg,image/png,image/webp,application/pdf",
                                        disabled: submitting(),
                                        class: "block w-full text-[13px] text-ink-soft file:mr-3 file:py-2 file:px-3 file:rounded-md file:border file:border-rule file:bg-bone-soft file:text-ink hover:file:bg-bone file:font-mono file:text-[12px] file:cursor-pointer cursor-pointer",
                                        onchange: move |e| async move {
                                            submit_error.set(None);
                                            let mut clear = move || {
                                                selected_proof_meta.set(None);
                                                selected_proof_bytes.set(None);
                                            };
                                            let Some(file) = e.files().into_iter().next() else {
                                                clear();
                                                return;
                                            };
                                            let name = file.name();
                                            // Prefer the browser-reported MIME (file picker hands
                                            // it to us); fall back to extension sniff for PDFs etc.
                                            // that some platforms label as octet-stream.
                                            let ct = match file.content_type().as_deref() {
                                                Some("image/jpeg") => Some("image/jpeg"),
                                                Some("image/png") => Some("image/png"),
                                                Some("image/webp") => Some("image/webp"),
                                                Some("application/pdf") => Some("application/pdf"),
                                                _ => proof_content_type(&name),
                                            };
                                            let Some(ct) = ct else {
                                                submit_error.set(Some(
                                                    "Receipt must be JPG, PNG, WebP, or PDF.".into(),
                                                ));
                                                clear();
                                                return;
                                            };
                                            if file.size() as usize > MAX_PROOF_BYTES {
                                                submit_error.set(Some(format!(
                                                    "Receipt is too large ({} MB). Max 10 MB.",
                                                    file.size() as usize / (1024 * 1024)
                                                )));
                                                clear();
                                                return;
                                            }
                                            let bytes = match file.read_bytes().await {
                                                Ok(b) => b.to_vec(),
                                                Err(err) => {
                                                    submit_error.set(Some(format!(
                                                        "Couldn't read the selected file: {err}"
                                                    )));
                                                    clear();
                                                    return;
                                                }
                                            };
                                            let size = bytes.len();
                                            selected_proof_meta.set(Some((name, ct, size)));
                                            selected_proof_bytes.set(Some(bytes));
                                        },
                                    }
                                    {selected_proof_meta().map(|(name, _, size)| {
                                        let kb = size.div_ceil(1024);
                                        rsx! {
                                            div {
                                                class: "mt-2 flex items-center gap-2 text-[12px] text-ink-soft font-mono",
                                                span { class: "truncate", "Staged: {name} · {kb} KB" }
                                                button {
                                                    r#type: "button",
                                                    disabled: submitting(),
                                                    onclick: move |_| {
                                                        selected_proof_meta.set(None);
                                                        selected_proof_bytes.set(None);
                                                    },
                                                    class: "text-evergreen hover:text-evergreen-deep underline underline-offset-2",
                                                    "remove"
                                                }
                                            }
                                        }
                                    })}
                                }

                                button {
                                    class: "mt-1 inline-flex items-center justify-center gap-2 bg-evergreen hover:bg-evergreen-deep disabled:opacity-60 disabled:cursor-not-allowed text-paper text-[14px] font-medium px-5 py-3 rounded-md transition-colors",
                                    disabled: submitting(),
                                    onclick: on_submit,
                                    if submitting() { "Recording…" } else { "Record payment" }
                                    if !submitting() {
                                        span { class: "text-[15px] leading-none", "→" }
                                    }
                                }

                                if let Some(msg) = submit_error() {
                                    p {
                                        class: "text-[12.5px] text-negative font-mono leading-[1.5]",
                                        "{msg}"
                                    }
                                }
                                p {
                                    class: "text-[11.5px] text-ink-faint font-mono leading-[1.55]",
                                    "Payments are capped at the dues amount per cycle. Submissions that push the period total over the cap are refused."
                                }
                            }
                        }
                    };

                    rsx! {
                        // Header — page version reserves the right rail for
                        // the cap microcopy; drawer stacks vertically.
                        if compact {
                            div { class: "mb-4",
                                p { class: "eyebrow", "MY PAYMENTS · PERIOD {period}" }
                                h2 { class: "mt-1.5 font-display text-[20px] font-semibold text-ink leading-tight",
                                    "What you owe this cycle"
                                }
                            }
                        } else {
                            div {
                                class: "flex flex-col gap-2 sm:flex-row sm:items-baseline sm:justify-between sm:gap-6 mb-5",
                                div {
                                    p { class: "eyebrow", "MY PAYMENTS · PERIOD {period}" }
                                    h2 { class: "mt-2 font-display text-[22px] sm:text-[24px] font-semibold text-ink leading-tight",
                                        "What you owe this cycle"
                                    }
                                }
                                p {
                                    class: "text-[12px] text-ink-faint font-mono sm:shrink-0 sm:text-right",
                                    "Cap is one cycle of dues. Partial payments add up."
                                }
                            }
                        }

                        // Card — vertical stack in compact, 2-col in page.
                        if compact {
                            div { class: "card p-5 flex flex-col gap-5",
                                {progress_block}
                                {stats_block}
                                div { class: "border-t border-rule-soft pt-5",
                                    {form_block}
                                }
                            }
                        } else {
                            div { class: "card p-6 grid grid-cols-1 lg:grid-cols-12 gap-8",
                                div { class: "lg:col-span-5 flex flex-col gap-5",
                                    {progress_block}
                                    {stats_block}
                                }
                                div { class: "lg:col-span-7",
                                    {form_block}
                                }
                            }
                        }

                        // ── Payment history this period ──────────────────────
                        div {
                            class: if compact { "mt-5" } else { "mt-6" },
                            p { class: "eyebrow mb-3", "THIS PERIOD'S PAYMENTS · {rows.len()}" }
                            if rows.is_empty() {
                                div {
                                    class: "card px-5 py-5 text-[13px] text-ink-soft",
                                    "No payments recorded yet for {period}."
                                }
                            } else {
                                div {
                                    class: "card overflow-hidden",
                                    div {
                                        class: "hidden sm:grid grid-cols-[1fr_auto_auto] gap-3 px-4 py-2.5 bg-bone-soft border-b border-rule text-[10.5px] text-ink-faint font-mono uppercase tracking-[0.12em]",
                                        span { "When · note" }
                                        span { class: "text-right", "Status" }
                                        span { class: "text-right", "Amount" }
                                    }
                                    for (i, row) in rows.iter().enumerate() {
                                        ContributionRowView {
                                            idx: i,
                                            row: row.clone(),
                                            currency: currency.clone(),
                                            slug: slug.clone(),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }}
        }
    }
}

#[component]
fn SmallFact(label: String, value: String) -> Element {
    rsx! {
        div {
            p { class: "eyebrow mb-1", "{label}" }
            p { class: "font-display text-[16px] text-ink font-light tnum tracking-[-0.005em]",
                "{value}"
            }
        }
    }
}

#[component]
fn ContributionRowView(
    idx: usize,
    row: Contribution,
    currency: String,
    /// Tenant slug — needed to resolve the proof URL via the gateway.
    slug: String,
) -> Element {
    let stripe = if idx % 2 == 1 {
        "bg-bone-soft/50"
    } else {
        "bg-paper"
    };
    let when = row
        .submitted_at
        .get(..16)
        .unwrap_or(&row.submitted_at)
        .replace('T', " ");
    let amount = fmt_money(row.amount_cents, &currency);
    let note = row.note.unwrap_or_default();
    let status_cls = match row.status.as_str() {
        "verified" => "pill pill-positive",
        "rejected" => "pill pill-negative",
        _ => "pill pill-neutral",
    };
    let proof_key = row.proof_key.clone();
    let mut proof_error: Signal<Option<String>> = use_signal(|| None);
    let mut proof_loading = use_signal(|| false);
    rsx! {
        div {
            class: "flex flex-col gap-2 sm:gap-4 sm:grid sm:grid-cols-[1fr_auto_auto] sm:items-center px-4 sm:px-5 py-3 border-b border-rule-soft last:border-b-0 {stripe}",
            div {
                // On mobile, lead with the amount + status pill so the row's
                // primary signal is readable at a glance; the desktop layout
                // keeps the conventional "when on the left, money on the right".
                div { class: "flex sm:hidden items-baseline justify-between gap-3 mb-0.5",
                    span { class: "font-display text-[16px] text-ink tnum", "{amount}" }
                    span { class: "{status_cls}", "{row.status}" }
                }
                p { class: "text-[13px] text-ink-soft sm:text-ink font-mono tnum", "{when} UTC" }
                if !note.is_empty() {
                    p { class: "text-[12.5px] text-ink-soft mt-0.5", "{note}" }
                }
                if let Some(pk) = proof_key {
                    div {
                        class: "mt-1 flex items-center gap-2",
                        button {
                            r#type: "button",
                            class: "text-[12px] font-mono text-evergreen hover:text-evergreen-deep underline underline-offset-2 disabled:opacity-50",
                            disabled: proof_loading(),
                            onclick: {
                                let slug = slug.clone();
                                let pk = pk.clone();
                                move |_| {
                                    let slug = slug.clone();
                                    let pk = pk.clone();
                                    async move {
                                        proof_error.set(None);
                                        proof_loading.set(true);
                                        match fetch_proof_url(&slug, &pk).await {
                                            Ok(url) => {
                                                if let Some(window) = web_sys::window() {
                                                    let _ = window.open_with_url_and_target(
                                                        &url, "_blank",
                                                    );
                                                }
                                            }
                                            Err(e) => proof_error.set(Some(e.to_string())),
                                        }
                                        proof_loading.set(false);
                                    }
                                }
                            },
                            if proof_loading() { "Opening receipt…" } else { "View receipt" }
                        }
                    }
                    if let Some(msg) = proof_error() {
                        p { class: "text-[11.5px] text-negative font-mono mt-0.5", "{msg}" }
                    }
                }
            }
            div { class: "hidden sm:block text-right",
                span { class: "{status_cls}", "{row.status}" }
            }
            div { class: "hidden sm:block text-right font-display text-[15px] text-ink tnum", "{amount}" }
        }
    }
}

// ─── Venture-wide pool (all members, selectable period) ───────────────────

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct PoolSummary {
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

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct PoolContribution {
    id: String,
    user_email: String,
    amount_cents: i64,
    status: String,
    note: Option<String>,
    submitted_at: String,
}

#[derive(Deserialize)]
struct PoolBody {
    summary: PoolSummary,
    contributions: Vec<PoolContribution>,
}

async fn fetch_pool(
    slug: String,
    period: Option<String>,
) -> Result<(PoolSummary, Vec<PoolContribution>), ApiError> {
    let path = match period {
        Some(p) if !p.is_empty() => {
            format!("/api/tenants/{slug}/contributions/pool?period={p}")
        }
        _ => format!("/api/tenants/{slug}/contributions/pool"),
    };
    let resp = authed(reqwest::Method::GET, &path)?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: PoolBody = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode pool: {e}")))?;
    Ok((body.summary, body.contributions))
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct PeriodsBody {
    current_period: String,
    cadence: String,
    periods: Vec<String>,
}

async fn fetch_periods(slug: String) -> Result<PeriodsBody, ApiError> {
    let resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{slug}/periods"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    resp.json::<PeriodsBody>()
        .await
        .map_err(|e| ApiError::Other(format!("decode periods: {e}")))
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct AccPoint {
    bucket: String,
    period_cents: i64,
    cumulative_cents: i64,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct CarryForwardSeed {
    amount_cents: i64,
    from_date: String,
    to_date: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct AccBody {
    #[allow(dead_code)]
    bucket: String,
    currency: String,
    #[serde(default)]
    carry_forward: Option<CarryForwardSeed>,
    series: Vec<AccPoint>,
}

async fn fetch_accumulation(slug: String, bucket: String) -> Result<AccBody, ApiError> {
    let path = format!("/api/tenants/{slug}/accumulation?bucket={bucket}");
    let resp = authed(reqwest::Method::GET, &path)?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    resp.json::<AccBody>()
        .await
        .map_err(|e| ApiError::Other(format!("decode accumulation: {e}")))
}

fn fmt_period_label(p: &str, cadence: &str) -> String {
    match cadence {
        "weekly" => p.replacen('-', " · ", 1),
        "yearly" => p.to_string(),
        _ => p.to_string(),
    }
}

#[component]
fn AuditLogPanel(slug: String) -> Element {
    // None means "let server pick the current period". A user choice flips
    // it to Some("2026-04") etc. Currency comes from the summary, not props,
    // so the table uses the same currency the gateway computed against.
    let mut selected_period = use_signal::<Option<String>>(|| None);

    let slug_for_periods = slug.clone();
    let periods = use_resource(move || {
        let s = slug_for_periods.clone();
        async move { fetch_periods(s).await }
    });

    let slug_for_pool = slug.clone();
    let pool = use_resource(move || {
        let s = slug_for_pool.clone();
        let p = selected_period();
        async move { fetch_pool(s, p).await }
    });

    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-12 max-w-[1140px] rise",
            style: "animation-delay: 0.28s",
            div {
                class: "card p-6",
                {match pool() {
                    None => rsx! {
                        p { class: "eyebrow mb-3", "AUDIT LOG" }
                        p { class: "text-[14px] text-ink-soft", "Reading the ledger…" }
                    },
                    Some(Err(e)) => {
                        let msg = e.to_string();
                        rsx! {
                            p { class: "eyebrow mb-3", "AUDIT LOG" }
                            p { class: "text-[13px] text-negative", "{msg}" }
                        }
                    }
                    Some(Ok((summary, contribs))) => rsx! {
                        AuditLogHeader {
                            summary: summary.clone(),
                            periods: periods(),
                            selected: selected_period(),
                            on_pick: move |p: String| {
                                if p.is_empty() {
                                    selected_period.set(None);
                                } else {
                                    selected_period.set(Some(p));
                                }
                            },
                        }
                        PoolTable {
                            currency: summary.currency.clone(),
                            rows: contribs,
                        }
                    }
                }}
            }
        }
    }
}

#[component]
fn AuditLogHeader(
    summary: PoolSummary,
    periods: Option<Result<PeriodsBody, ApiError>>,
    selected: Option<String>,
    on_pick: EventHandler<String>,
) -> Element {
    let label = fmt_period_label(&summary.period, &summary.cadence);
    let dropdown_value = selected.clone().unwrap_or_default();
    let options: Vec<String> = match &periods {
        Some(Ok(b)) => b.periods.clone(),
        _ => vec![summary.period.clone()],
    };
    rsx! {
        div {
            class: "flex flex-wrap items-end justify-between gap-4 mb-5",
            div {
                p { class: "eyebrow mb-1", "AUDIT LOG · {label}" }
                p {
                    class: "text-[13px] text-ink-soft",
                    "Every payment recorded into this period — append-only, period-locked once the cycle closes."
                }
            }
            div {
                class: "flex items-center gap-2",
                label {
                    class: "text-[11px] uppercase tracking-[0.1em] text-ink-faint",
                    "Period"
                }
                select {
                    class: "text-[13px] font-mono tnum bg-paper border border-rule rounded-md px-3 py-1.5 hover:border-evergreen/40 focus:border-evergreen/60 focus:outline-none",
                    value: "{dropdown_value}",
                    onchange: move |e| on_pick.call(e.value()),
                    // "" means "use server's current period". Keep it as the
                    // default option so the user can always snap back.
                    option { value: "", "Latest ({summary.period})" }
                    for p in options.iter() {
                        option { value: "{p}", "{p}" }
                    }
                }
            }
        }
    }
}

#[component]
fn PoolBar(summary: PoolSummary) -> Element {
    // Ratio against the per-period collective target (dues × member_count).
    // When dues_amount = 0 (donation-style), we use the largest paid value
    // we've seen so the bar is at least visible — otherwise show empty.
    let pct: f32 = if summary.target_cents > 0 {
        ((summary.paid_cents as f64 / summary.target_cents as f64) * 100.0)
            .clamp(0.0, 100.0) as f32
    } else if summary.paid_cents > 0 {
        100.0
    } else {
        0.0
    };
    let pct_label = format!("{pct:.1}%");
    let bar_tone = if summary.target_cents == 0 {
        "bg-evergreen/60"
    } else if summary.remaining_cents == 0 {
        "bg-positive"
    } else if summary.paid_cents > 0 {
        "bg-amber"
    } else {
        "bg-bone-soft"
    };
    let width_style = format!("width: {pct:.2}%;");
    rsx! {
        div { class: "mb-2 flex justify-between text-[12px] text-ink-soft font-mono uppercase tracking-[0.08em]",
            span { "Collected" }
            span { "{pct_label}" }
        }
        div {
            class: "relative h-7 w-full bg-bone-soft rounded-md overflow-hidden border border-rule",
            div {
                class: "absolute inset-y-0 left-0 {bar_tone} transition-[width] duration-500 ease-out",
                style: "{width_style}",
            }
            // Centered overlay text — always visible regardless of fill.
            div {
                class: "absolute inset-0 flex items-center justify-center text-[12px] font-mono tnum text-ink",
                "{fmt_money(summary.paid_cents, &summary.currency)} of {fmt_money(summary.target_cents, &summary.currency)}"
            }
        }
    }
}

// ─── Holding summary — top-of-Overview balance sheet headline ─────────────

#[component]
fn HoldingSummary(slug: String, currency: String, is_admin: bool) -> Element {
    // Two parallel fetches: pool gives this-period progress + member counts;
    // accumulation gives lifetime cumulative (last point of the series).
    let slug_for_pool = slug.clone();
    let pool = use_resource(move || {
        let s = slug_for_pool.clone();
        async move { fetch_pool(s, None).await }
    });

    let slug_for_acc = slug.clone();
    let acc = use_resource(move || {
        let s = slug_for_acc.clone();
        async move { fetch_accumulation(s, "auto".to_string()).await }
    });

    // Lifetime cents: prefer the last series point (already includes the
    // carry-forward seed at offset). If the series is empty but a seed exists,
    // the lifetime IS the seed.
    let acc_snapshot = acc();
    let lifetime_cents: Option<i64> = match &acc_snapshot {
        Some(Ok(body)) => body
            .series
            .last()
            .map(|p| p.cumulative_cents)
            .or_else(|| body.carry_forward.as_ref().map(|c| c.amount_cents)),
        _ => None,
    };
    let lifetime_label = match lifetime_cents {
        Some(c) => fmt_money(c, &currency),
        None => "—".to_string(),
    };
    let carry_forward: Option<CarryForwardSeed> = match &acc_snapshot {
        Some(Ok(body)) => body.carry_forward.clone(),
        _ => None,
    };

    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-2 pb-10 max-w-[1140px] rise",
            style: "animation-delay: 0.06s",

            div {
                class: "card p-6 lg:p-8",

                // Top eyebrow + admin escape hatch.
                div { class: "flex items-baseline justify-between mb-6 flex-wrap gap-3",
                    p { class: "eyebrow", "HOLDING · BALANCE SHEET" }
                    if is_admin {
                        a {
                            href: "/ventures/{slug}/manage",
                            class: "text-[11.5px] text-ink-faint hover:text-evergreen font-mono tracking-[0.1em] uppercase border-b border-transparent hover:border-evergreen/50 transition-colors",
                            "Member roster →"
                        }
                    }
                }

                // Main grid: lifetime (left, dominant) + this-period progress (right).
                div { class: "grid grid-cols-1 lg:grid-cols-12 gap-8 items-end",

                    // Lifetime collected — the headline number.
                    div { class: "lg:col-span-5",
                        p { class: "text-[11px] uppercase tracking-[0.16em] text-ink-faint font-mono", "Total collected · lifetime" }
                        p {
                            class: "mt-3 font-display text-[clamp(2.5rem,5vw,3.75rem)] font-light leading-[1.0] text-ink tnum tracking-[-0.015em]",
                            "{lifetime_label}"
                        }
                        p { class: "mt-3 text-[12.5px] text-ink-soft",
                            "Sum of every verified payment since the venture opened."
                        }
                        if let Some(cf) = carry_forward.as_ref() {
                            div {
                                class: "mt-4 rounded-md border border-rule-soft bg-bone-soft/60 px-4 py-3",
                                p {
                                    class: "text-[10.5px] uppercase tracking-[0.12em] text-ink-faint font-mono",
                                    "INCLUDES CARRY-FORWARD SEED"
                                }
                                p {
                                    class: "mt-1.5 font-display text-[18px] text-ink tnum",
                                    "{fmt_money(cf.amount_cents, &currency)}"
                                }
                                p {
                                    class: "mt-1 text-[12px] text-ink-soft font-mono tnum",
                                    "Accumulated {cf.from_date} → {cf.to_date}"
                                }
                                if let Some(note) = cf.note.as_ref().filter(|n| !n.is_empty()) {
                                    p {
                                        class: "mt-1.5 text-[12px] text-ink-soft italic leading-relaxed",
                                        "“{note}”"
                                    }
                                }
                            }
                        }
                    }

                    // Right: this-period bar + key facts.
                    div { class: "lg:col-span-7",
                        {match pool() {
                            None => rsx! {
                                div {
                                    class: "well p-5 text-[13px] text-ink-soft",
                                    "Reading current period…"
                                }
                            },
                            Some(Err(e)) => {
                                let msg = e.to_string();
                                rsx! {
                                    div {
                                        class: "well p-5 text-[13px] text-negative font-mono",
                                        "{msg}"
                                    }
                                }
                            }
                            Some(Ok((summary, _))) => {
                                let label = fmt_period_label(&summary.period, &summary.cadence);
                                let outstanding = summary.member_count.saturating_sub(summary.settled_count);
                                rsx! {
                                    div { class: "well p-5",
                                        div { class: "mb-4 flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4",
                                            p { class: "eyebrow", "THIS PERIOD · {label}" }
                                            p {
                                                class: "text-[11px] text-ink-faint font-mono",
                                                "Period locks at cycle close"
                                            }
                                        }
                                        PoolBar { summary: summary.clone() }

                                        div {
                                            class: "mt-5 grid grid-cols-3 gap-3 text-[12.5px]",
                                            PoolFact {
                                                label: "Members",
                                                value: format!("{}", summary.member_count),
                                            }
                                            PoolFact {
                                                label: "Settled",
                                                value: format!("{} of {}", summary.settled_count, summary.member_count),
                                            }
                                            PoolFact {
                                                label: "Outstanding",
                                                value: format!("{}", outstanding),
                                            }
                                        }
                                    }
                                }
                            }
                        }}
                    }
                }
            }
        }
    }
}

#[component]
fn PoolFact(label: String, value: String) -> Element {
    rsx! {
        div {
            class: "rounded-md bg-bone-soft/60 px-3 py-2 border border-rule-soft",
            p { class: "text-[10.5px] uppercase tracking-[0.1em] text-ink-faint", "{label}" }
            p { class: "mt-1 font-display text-ink tnum", "{value}" }
        }
    }
}

#[component]
fn PoolTable(currency: String, rows: Vec<PoolContribution>) -> Element {
    if rows.is_empty() {
        return rsx! {
            div {
                class: "mt-6 rounded-md border border-rule-soft bg-bone-soft/40 px-5 py-6 text-center text-[13px] text-ink-soft",
                "No payments recorded for this period yet."
            }
        };
    }
    rsx! {
        div {
            class: "mt-6 rounded-md border border-rule-soft overflow-hidden",
            div {
                class: "hidden sm:grid grid-cols-[2fr_1.4fr_auto_auto] gap-4 px-5 py-2.5 bg-bone-soft text-[10.5px] uppercase tracking-[0.1em] text-ink-faint font-mono",
                span { "Member" }
                span { "Submitted" }
                span { class: "text-right", "Status" }
                span { class: "text-right", "Amount" }
            }
            for (idx, r) in rows.iter().enumerate() {
                PoolRow { row: r.clone(), currency: currency.clone(), idx }
            }
        }
    }
}

#[component]
fn PoolRow(row: PoolContribution, currency: String, idx: usize) -> Element {
    let stripe = if idx % 2 == 0 { "bg-paper" } else { "bg-bone-soft/30" };
    let when = row.submitted_at.split('T').next().unwrap_or(&row.submitted_at);
    let amount = fmt_money(row.amount_cents, &currency);
    let note = row.note.unwrap_or_default();
    let status_cls = match row.status.as_str() {
        "verified" => "pill pill-positive",
        "rejected" => "pill pill-negative",
        _ => "pill pill-neutral",
    };
    rsx! {
        div {
            class: "flex flex-col gap-1.5 sm:gap-4 sm:grid sm:grid-cols-[2fr_1.4fr_auto_auto] sm:items-center px-4 sm:px-5 py-3 border-t border-rule-soft {stripe}",
            // Mobile-only header line: amount + status, leading the row.
            div { class: "flex sm:hidden items-baseline justify-between gap-3",
                span { class: "font-display text-[16px] text-ink tnum", "{amount}" }
                span { class: "{status_cls}", "{row.status}" }
            }
            div {
                p { class: "text-[13px] text-ink truncate", "{row.user_email}" }
                if !note.is_empty() {
                    p { class: "text-[12px] text-ink-soft mt-0.5 truncate", "{note}" }
                }
            }
            div { class: "text-[12.5px] sm:text-[13px] text-ink-soft font-mono tnum", "{when}" }
            div { class: "hidden sm:block text-right",
                span { class: "{status_cls}", "{row.status}" }
            }
            div { class: "hidden sm:block text-right font-display text-[15px] text-ink tnum", "{amount}" }
        }
    }
}

// ─── Cumulative line chart (charming + ECharts) ───────────────────────────

#[component]
fn AccumulationChart(slug: String, currency: String) -> Element {
    // Bucket signal drives the request. "auto" → server decides (year if ≥1y
    // else month). "year" / "month" force it.
    let mut bucket = use_signal(|| "auto".to_string());
    // Frontend window: ALL, 1Y, 6M. Used to slice the returned series tail.
    let mut window_choice = use_signal(|| "ALL".to_string());

    let slug_for_load = slug.clone();
    let data = use_resource(move || {
        let s = slug_for_load.clone();
        let b = bucket();
        async move { fetch_accumulation(s, b).await }
    });

    // Stable id per slug so two ventures rendered in the same DOM tree don't
    // collide on echarts.init().
    let chart_id = format!("sharam-accum-{}", slug);
    let chart_id_for_div = chart_id.clone();

    // Effect: every time `data` or `window_choice` changes, re-render.
    let id_for_render = chart_id.clone();
    let curr_for_render = currency.clone();
    use_effect(move || {
        // Subscribe to both signals.
        let snap = data.read().clone();
        let win = window_choice();
        let Some(Ok(body)) = snap else { return };
        render_accumulation_chart(&id_for_render, &curr_for_render, &body, &win);
    });

    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-16 max-w-[1140px] rise",
            style: "animation-delay: 0.22s",
            div { class: "card p-4 sm:p-6 overflow-hidden",
                div { class: "flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-end sm:justify-between sm:gap-4 mb-4",
                    div {
                        p { class: "eyebrow mb-1", "ACCUMULATION" }
                        p {
                            class: "text-[13px] text-ink-soft",
                            "Cumulative collected across the venture's lifetime."
                        }
                    }
                    div {
                        class: "flex items-center gap-2 sm:gap-3 flex-wrap",
                        BucketToggle {
                            value: bucket(),
                            on_pick: move |b: String| bucket.set(b),
                        }
                        WindowToggle {
                            value: window_choice(),
                            on_pick: move |w: String| window_choice.set(w),
                        }
                    }
                }
                {match data() {
                    None => rsx! {
                        p { class: "text-[13px] text-ink-soft py-12 text-center", "Drawing chart…" }
                    },
                    Some(Err(e)) => {
                        let msg = e.to_string();
                        rsx! { p { class: "text-[13px] text-negative", "{msg}" } }
                    }
                    Some(Ok(body)) if body.series.is_empty() => {
                        let cf = body.carry_forward.clone();
                        let curr = body.currency.clone();
                        rsx! {
                            div {
                                class: "py-12 text-center text-[13px] text-ink-soft border border-rule-soft rounded-md bg-bone-soft/40",
                                if let Some(c) = cf {
                                    p {
                                        "Starting balance from off-platform: "
                                        span { class: "font-mono text-ink", "{fmt_money(c.amount_cents, &curr)}" }
                                        " (accumulated {c.from_date} → {c.to_date})."
                                    }
                                    p { class: "mt-2",
                                        "The chart will fill in once members start contributing on Sharam."
                                    }
                                } else {
                                    "No payments recorded yet — the chart will appear once members start contributing."
                                }
                            }
                        }
                    }
                    Some(Ok(body)) => {
                        let cf_caption = body.carry_forward.as_ref().map(|c| {
                            (
                                c.amount_cents,
                                c.from_date.clone(),
                                c.to_date.clone(),
                                body.currency.clone(),
                            )
                        });
                        rsx! {
                            div {
                                id: "{chart_id_for_div}",
                                // Width matches the card's content area at lg
                                // breakpoint; height keeps the line readable.
                                style: "width: 100%; height: 320px;",
                            }
                            if let Some((amt, from, to, curr)) = cf_caption {
                                p {
                                    class: "mt-3 text-[12px] text-ink-soft text-center font-mono",
                                    span { class: "inline-block w-3 h-px bg-amber align-middle mr-2" }
                                    "Dashed line marks carry-forward seed of "
                                    span { class: "text-ink", "{fmt_money(amt, &curr)}" }
                                    " accumulated {from} → {to}. Contributions stack above."
                                }
                            }
                        }
                    }
                }}
            }
        }
    }
}

fn render_accumulation_chart(id: &str, currency: &str, body: &AccBody, window: &str) {
    use charming::{
        component::{Axis, Grid},
        element::{
            AreaStyle, AxisType, Color, ColorStop, ItemStyle, LineStyle, LineStyleType, Tooltip,
            Trigger,
        },
        series::Line,
        Chart, WasmRenderer,
    };

    // Slice the tail by window choice. ALL = everything; 1Y = last 12; 6M
    // = last 6. Counts are interpreted in whatever bucket the server picked.
    let slice: &[AccPoint] = match window {
        "1Y" => &body.series[body.series.len().saturating_sub(12)..],
        "6M" => &body.series[body.series.len().saturating_sub(6)..],
        _ => &body.series[..],
    };

    let labels: Vec<String> = slice.iter().map(|p| p.bucket.clone()).collect();
    // Convert cents → major units for axis readability. Chart shows currency
    // in the tooltip; axis shows naked numbers.
    let cumulative: Vec<i64> = slice.iter().map(|p| p.cumulative_cents / 100).collect();

    // Carry-forward seed in major units. Drawn as a dashed flat reference
    // line so the user can see "everything below the dash was already in
    // the kitty before Sharam; everything above is post-platform flow".
    let seed_major: Option<i64> = body.carry_forward.as_ref().map(|c| c.amount_cents / 100);

    let mut chart = Chart::new()
        .grid(
            Grid::new()
                .left("3%")
                .right("4%")
                .bottom("8%")
                .contain_label(true),
        )
        .tooltip(Tooltip::new().trigger(Trigger::Axis))
        .x_axis(
            Axis::new()
                .type_(AxisType::Category)
                .boundary_gap(false)
                .data(labels.clone()),
        )
        .y_axis(
            Axis::new()
                .type_(AxisType::Value)
                .name(format!("Cumulative ({currency})")),
        )
        .series(
            Line::new()
                .name("Cumulative")
                .smooth(0.3)
                .show_symbol(false)
                .line_style(LineStyle::new().width(2.0).color("#1f4d3d"))
                .item_style(ItemStyle::new().color("#1f4d3d"))
                .area_style(AreaStyle::new().color(Color::LinearGradient {
                    x: 0.0,
                    y: 0.0,
                    x2: 0.0,
                    y2: 1.0,
                    color_stops: vec![
                        ColorStop::new(0.0, "rgba(31, 77, 61, 0.45)"),
                        ColorStop::new(1.0, "rgba(31, 77, 61, 0.04)"),
                    ],
                }))
                .data(cumulative),
        );

    if let Some(seed) = seed_major {
        let flat: Vec<i64> = vec![seed; labels.len()];
        chart = chart.series(
            Line::new()
                .name("Carry-forward seed")
                .show_symbol(false)
                .line_style(
                    LineStyle::new()
                        .width(1.5)
                        .color("#b78a3c")
                        .type_(LineStyleType::Dashed),
                )
                .item_style(ItemStyle::new().color("#b78a3c"))
                .data(flat),
        );
    }

    // Read the container's actual client width so the chart matches its parent.
    // Falls back to 960 only if the element isn't in the DOM yet (shouldn't
    // happen because this runs from a use_effect post-mount, but defensive).
    let width: u32 = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
        .map(|el| el.client_width().max(0) as u32)
        .filter(|w| *w > 0)
        .unwrap_or(960);

    let renderer = WasmRenderer::new(width, 320);
    if let Err(e) = renderer.render(id, &chart) {
        web_sys::console::warn_1(&format!("accumulation chart render: {e:?}").into());
    }
}

#[component]
fn BucketToggle(value: String, on_pick: EventHandler<String>) -> Element {
    let opts = [("auto", "Auto"), ("month", "Month"), ("year", "Year")];
    rsx! {
        div { class: "inline-flex rounded-md border border-rule overflow-hidden text-[12px] font-mono",
            for (val, label) in opts.iter() {
                {
                    let active = value.as_str() == *val;
                    let cls = if active {
                        "px-3 py-1.5 bg-evergreen text-paper"
                    } else {
                        "px-3 py-1.5 bg-paper text-ink-soft hover:bg-bone-soft"
                    };
                    let v = val.to_string();
                    rsx! {
                        button {
                            r#type: "button",
                            class: "{cls}",
                            onclick: move |_| on_pick.call(v.clone()),
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn WindowToggle(value: String, on_pick: EventHandler<String>) -> Element {
    let opts = ["6M", "1Y", "ALL"];
    rsx! {
        div { class: "inline-flex rounded-md border border-rule overflow-hidden text-[12px] font-mono",
            for val in opts.iter() {
                {
                    let active = value.as_str() == *val;
                    let cls = if active {
                        "px-3 py-1.5 bg-evergreen text-paper"
                    } else {
                        "px-3 py-1.5 bg-paper text-ink-soft hover:bg-bone-soft"
                    };
                    let v = val.to_string();
                    rsx! {
                        button {
                            r#type: "button",
                            class: "{cls}",
                            onclick: move |_| on_pick.call(v.clone()),
                            "{val}"
                        }
                    }
                }
            }
        }
    }
}
