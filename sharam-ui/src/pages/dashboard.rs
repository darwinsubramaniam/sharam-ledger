use dioxus::prelude::*;
use serde::Deserialize;

use crate::api::{authed, into_api_error, ApiError};
use crate::pages::sidenav::{Sidenav, Venture, VentureCtx};

// ── Wire shape (per-venture settings) ───────────────────────────────────
#[derive(Deserialize, Debug, Clone, PartialEq)]
struct Settings {
    display_name: String,
    timezone: String,
    currency: String,
    cadence: String,
    dues_amount_cents: i64,
    current_period: String,
}

#[derive(Deserialize)]
struct SettingsBody {
    settings: Settings,
}

async fn fetch_settings(slug: String) -> Result<Settings, ApiError> {
    let path = format!("/api/tenants/{slug}/settings");
    let resp = authed(reqwest::Method::GET, &path)?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: SettingsBody = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))?;
    Ok(body.settings)
}

// ── Helpers ─────────────────────────────────────────────────────────────
fn fmt_money(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.abs();
    let major = abs / 100;
    let minor = abs % 100;
    let raw = major.to_string();
    let mut grouped = String::new();
    for (i, c) in raw.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let major_str: String = grouped.chars().rev().collect();
    format!("{sign}{major_str}.{minor:02} {currency}")
}

fn role_pill(role: &str) -> &'static str {
    match role {
        "owner" => "pill pill-evergreen",
        "treasurer" => "pill pill-amber",
        _ => "pill pill-neutral",
    }
}

fn cadence_word(c: &str) -> &'static str {
    match c {
        "weekly" => "Each ISO week",
        "yearly" => "Each calendar year",
        _ => "Each calendar month",
    }
}

// ── Page ────────────────────────────────────────────────────────────────
#[component]
pub fn Dashboard() -> Element {
    rsx! {
        Sidenav { active: "ventures".to_string(),
            DashboardBody {}
        }
    }
}

#[component]
fn DashboardBody() -> Element {
    let ctx = use_context::<VentureCtx>();
    let ventures_opt = (ctx.ventures)();
    let selected_now = (ctx.selected)();
    let err = (ctx.error)();

    let detail = use_resource(move || {
        let sel = ctx.selected;
        async move {
            match sel() {
                Some(slug) => Some(fetch_settings(slug).await),
                None => None,
            }
        }
    });
    let detail_value = detail();

    rsx! {
        {match (ventures_opt, err) {
            (None, _) => rsx! { LoadingSection {} },
            (Some(_), Some(ApiError::NotSignedIn)) | (Some(_), Some(ApiError::Unauthorized)) => rsx! { SignInSection {} },
            (Some(_), Some(e)) => {
                let msg = e.to_string();
                rsx! { ErrorSection { message: msg } }
            },
            (Some(list), None) if list.is_empty() => rsx! { EmptySection {} },
            (Some(list), None) => {
                let active = selected_now
                    .as_ref()
                    .and_then(|s| list.iter().find(|v| &v.slug == s).cloned())
                    .or_else(|| list.first().cloned());
                rsx! {
                    DashboardContent {
                        active: active,
                        detail: detail_value.and_then(|d| d),
                    }
                }
            },
        }}
    }
}

// ── Content (header + holdings + activity) ──────────────────────────────
#[component]
fn DashboardContent(
    active: Option<Venture>,
    detail: Option<Result<Settings, ApiError>>,
) -> Element {
    let title: String = active
        .as_ref()
        .map(|v| v.display_name.clone())
        .unwrap_or_else(|| "Pick a venture".to_string());

    rsx! {
        // ── Title ──────────────────────────────────────────────────────
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-4 max-w-[1140px] rise",
            p { class: "eyebrow mb-4", "DASHBOARD" }
            h1 {
                class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.08] text-ink",
                "{title}"
            }
            if active.is_some() {
                p {
                    class: "mt-3 text-[15px] text-ink-soft font-light leading-[1.55] max-w-2xl",
                    "Your holdings, the period the ledger sits in, and recent activity. Switch venture from the sidebar at any time."
                }
            } else {
                p {
                    class: "mt-3 text-[15px] text-ink-soft font-light leading-[1.55] max-w-2xl",
                    "Choose a venture from the picker in the sidebar to see its holdings summary and activity."
                }
            }
        }

        // ── Holdings + Activity ───────────────────────────────────────
        if let Some(v) = active.as_ref() {
            HoldingsAndActivity { venture: v.clone(), detail: detail }
        } else {
            section {
                class: "px-4 sm:px-6 lg:px-12 pb-16 max-w-[1140px] rise",
                style: "animation-delay: 0.05s",
                div {
                    class: "card p-10 text-center",
                    p { class: "eyebrow mb-3", "NO VENTURE SELECTED" }
                    h2 {
                        class: "font-display text-[20px] font-semibold text-ink",
                        "Pick one from the sidebar"
                    }
                    p {
                        class: "mt-3 text-[14px] text-ink-soft max-w-md mx-auto",
                        "The active venture drives everything on this page. Select one and the ledger will populate."
                    }
                }
            }
        }
    }
}

// ── Holdings + Activity ─────────────────────────────────────────────────
#[component]
fn HoldingsAndActivity(venture: Venture, detail: Option<Result<Settings, ApiError>>) -> Element {
    let role_cls = role_pill(&venture.role);
    let joined = venture.created_at.get(..10).unwrap_or("").to_string();
    let role_lower = venture.role.clone();

    let (period_value, period_sub, dues_value, dues_sub, tz_caption): (
        String,
        String,
        String,
        String,
        String,
    ) = match detail.as_ref() {
        Some(Ok(s)) => (
            s.current_period.clone(),
            cadence_word(&s.cadence).to_string(),
            fmt_money(s.dues_amount_cents, &s.currency),
            s.currency.clone(),
            s.timezone.clone(),
        ),
        Some(Err(_)) => (
            "—".to_string(),
            "couldn't reach the venture".to_string(),
            "—".to_string(),
            "—".to_string(),
            "—".to_string(),
        ),
        None => (
            "···".to_string(),
            "loading".to_string(),
            "···".to_string(),
            "loading".to_string(),
            "syncing".to_string(),
        ),
    };

    rsx! {
        // ── Holdings ───────────────────────────────────────────────────
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-2 max-w-[1140px] rise",
            style: "animation-delay: 0.1s",
            div {
                class: "flex items-baseline justify-between mb-5 gap-4 flex-wrap",
                div {
                    class: "flex items-center gap-3",
                    p { class: "eyebrow", "HOLDINGS · SUMMARY" }
                    span { class: "{role_cls}", "{role_lower}" }
                }
                p {
                    class: "text-[12px] text-ink-faint font-mono tracking-[0.06em] tnum",
                    "{tz_caption}"
                }
            }

            div {
                class: "grid grid-cols-1 sm:grid-cols-3 gap-px bg-rule rounded-xl overflow-hidden border border-rule",

                // Active period
                div {
                    class: "bg-paper p-5",
                    p { class: "eyebrow mb-3", "ACTIVE PERIOD" }
                    p {
                        class: "font-display text-[1.65rem] font-light text-ink leading-[1.05] tnum tracking-[-0.01em]",
                        "{period_value}"
                    }
                    p {
                        class: "mt-2 text-[12px] text-ink-faint font-mono tracking-[0.04em]",
                        "{period_sub}"
                    }
                }

                // Dues per period
                div {
                    class: "bg-paper p-5",
                    p { class: "eyebrow mb-3", "DUES PER PERIOD" }
                    p {
                        class: "font-display text-[1.65rem] font-light text-ink leading-[1.05] tnum tracking-[-0.01em]",
                        "{dues_value}"
                    }
                    p {
                        class: "mt-2 text-[12px] text-ink-faint font-mono tracking-[0.04em]",
                        "{dues_sub}"
                    }
                }

                // Member since
                div {
                    class: "bg-paper p-5",
                    p { class: "eyebrow mb-3", "MEMBER SINCE" }
                    p {
                        class: "font-display text-[1.65rem] font-light text-ink leading-[1.05] tnum tracking-[-0.01em]",
                        if joined.is_empty() { "—" } else { "{joined}" }
                    }
                    p {
                        class: "mt-2 text-[12px] text-ink-faint font-mono tracking-[0.04em]",
                        "you joined"
                    }
                }
            }
        }

        // ── Activity ───────────────────────────────────────────────────
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px] rise",
            style: "animation-delay: 0.15s",
            div {
                class: "flex flex-col gap-1 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4 mb-5",
                p { class: "eyebrow", "ACTIVITY · LATEST" }
                p {
                    class: "text-[12px] text-ink-faint font-mono tracking-[0.06em] tnum",
                    "switch ventures from the sidebar"
                }
            }
            ActivityFeed { venture: venture, detail: detail }
        }
    }
}

// ── Activity ────────────────────────────────────────────────────────────
#[derive(Clone, PartialEq)]
struct ActivityEntry {
    kind: String,
    title: String,
    body: String,
    stamp: String,
}

#[component]
fn ActivityFeed(venture: Venture, detail: Option<Result<Settings, ApiError>>) -> Element {
    let joined = venture.created_at.get(..10).unwrap_or("").to_string();
    let role_lower = venture.role.clone();

    let mut entries: Vec<ActivityEntry> = Vec::new();

    if let Some(Ok(s)) = detail.as_ref() {
        let dues = fmt_money(s.dues_amount_cents, &s.currency);
        entries.push(ActivityEntry {
            kind: "period".into(),
            title: format!("Period {} is open", s.current_period),
            body: format!(
                "{} cadence; each contribution at {}. Submissions remain mutable until close, then harden.",
                s.cadence, dues
            ),
            stamp: format!("now · {}", s.timezone),
        });
        entries.push(ActivityEntry {
            kind: "settings".into(),
            title: "Cadence & dues synced".into(),
            body: format!(
                "Settings show {} per {} period in {}.",
                dues, s.cadence, s.currency
            ),
            stamp: "synced".into(),
        });
    }

    entries.push(ActivityEntry {
        kind: "membership".into(),
        title: format!("You joined as {role_lower}"),
        body: "Your seat in this ledger was granted via accepted invite. Removal is not retroactive — past contributions stay on record.".into(),
        stamp: if joined.is_empty() {
            "—".into()
        } else {
            format!("on {joined}")
        },
    });

    rsx! {
        if entries.is_empty() {
            div {
                class: "card p-8 text-center text-ink-soft text-[14px]",
                "No activity to surface yet."
            }
        } else {
            ol {
                class: "card divide-y divide-rule-soft overflow-hidden",
                for (i, e) in entries.iter().enumerate() {
                    ActivityRow { idx: i, entry: e.clone() }
                }
            }
            p {
                class: "mt-4 text-[12px] text-ink-faint font-mono leading-[1.6] max-w-2xl",
                "Live contribution submissions, period locks, and audit notes will populate here once the activity endpoint ships. For now: what is true today, derived from settings and your membership row."
            }
        }
    }
}

#[component]
fn ActivityRow(idx: usize, entry: ActivityEntry) -> Element {
    let dot_cls = match entry.kind.as_str() {
        "period" => "h-2.5 w-2.5 rounded-full bg-evergreen border border-evergreen-deep",
        "settings" => "h-2.5 w-2.5 rounded-full bg-amber/80 border border-amber",
        "membership" => "h-2.5 w-2.5 rounded-full bg-positive/85 border border-positive",
        _ => "h-2.5 w-2.5 rounded-full bg-bone-soft border border-rule",
    };
    let no = format!("{:02}", idx + 1);
    rsx! {
        li {
            class: "px-5 py-4 grid gap-4 items-start grid-cols-[auto_1fr_auto] hover:bg-bone-soft/40 transition-colors",
            div {
                class: "flex items-center gap-3 pt-1 shrink-0",
                span { class: "font-mono text-[11px] text-ink-faint tracking-[0.1em]", "{no}" }
                span { class: "{dot_cls}" }
            }
            div {
                class: "min-w-0",
                p { class: "font-display text-[15.5px] text-ink leading-[1.3] font-medium", "{entry.title}" }
                p { class: "mt-1 text-[13px] text-ink-soft leading-[1.55] font-light", "{entry.body}" }
            }
            span {
                class: "font-mono text-[11px] text-ink-faint tracking-[0.06em] tnum whitespace-nowrap pt-1",
                "{entry.stamp}"
            }
        }
    }
}

// ── States ──────────────────────────────────────────────────────────────
#[component]
fn LoadingSection() -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px] rise",
            p { class: "eyebrow mb-4", "DASHBOARD" }
            h1 {
                class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.08] text-ink",
                "Loading your ventures"
            }
            div {
                class: "mt-8 card p-8 text-center text-ink-soft text-[14px]",
                "Walking the directory…"
            }
        }
    }
}

#[component]
fn SignInSection() -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px] rise",
            div {
                class: "card p-8",
                p { class: "eyebrow mb-2", "SESSION EXPIRED" }
                h2 { class: "font-display text-[20px] font-semibold text-ink", "You're not signed in" }
                p { class: "mt-2 text-[14px] text-ink-soft",
                    "Your sign-in session expired or was never set up. Sign in again to see your ventures."
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
fn ErrorSection(message: String) -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px] rise",
            div {
                class: "card p-8",
                p { class: "eyebrow !text-negative mb-2", "ERROR" }
                p { class: "text-[14px] text-ink-soft", "{message}" }
            }
        }
    }
}

#[component]
fn EmptySection() -> Element {
    rsx! {
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-10 pb-6 max-w-[1140px] rise",
            p { class: "eyebrow mb-4", "DASHBOARD" }
            h1 {
                class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.08] text-ink",
                "Open your first ledger"
            }
            p {
                class: "mt-3 text-[15px] text-ink-soft font-light leading-[1.55] max-w-2xl",
                "You haven't joined any ventures and you haven't created one yet. Create one and you become its sole owner."
            }
        }
        section {
            class: "px-4 sm:px-6 lg:px-12 pb-16 max-w-[1140px] rise",
            style: "animation-delay: 0.05s",
            div {
                class: "card p-10 text-center",
                p { class: "eyebrow mb-3", "NO VENTURES YET" }
                h2 { class: "font-display text-[22px] font-semibold text-ink", "Start a venture" }
                p {
                    class: "mt-3 text-[14px] text-ink-soft max-w-md mx-auto",
                    "Pool patient capital. Track contributions. Lock periods that closed."
                }
                a {
                    href: "/tenants/new",
                    class: "mt-6 inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper text-[14px] font-medium px-5 py-2.5 rounded-md transition-colors",
                    "Create a venture"
                    span { class: "text-[16px] leading-none", "→" }
                }
            }
        }
    }
}
