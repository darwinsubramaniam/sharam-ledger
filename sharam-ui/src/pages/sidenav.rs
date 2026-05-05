use dioxus::prelude::*;
use serde::Deserialize;

use crate::api::{authed, into_api_error, sign_out, ApiError};

const STORAGE_KEY: &str = "sharam_active_venture";

// ── Wire shape ──────────────────────────────────────────────────────────
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Venture {
    pub slug: String,
    pub display_name: String,
    pub role: String,
    pub created_at: String,
}

#[derive(Deserialize)]
struct VenturesBody {
    ventures: Vec<Venture>,
}

pub async fn fetch_ventures() -> Result<Vec<Venture>, ApiError> {
    let resp = authed(reqwest::Method::GET, "/api/me/ventures")?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: VenturesBody = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))?;
    Ok(body.ventures)
}

// ── Context ─────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
pub struct VentureCtx {
    pub selected: Signal<Option<String>>,
    pub ventures: Signal<Option<Vec<Venture>>>, // None = still loading
    pub error: Signal<Option<ApiError>>,
}

// ── Helpers ─────────────────────────────────────────────────────────────
fn read_stored() -> Option<String> {
    let win = web_sys::window()?;
    let storage = win.local_storage().ok().flatten()?;
    let v = storage.get_item(STORAGE_KEY).ok().flatten()?;
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn write_stored(slug: Option<&str>) {
    if let Some(win) = web_sys::window() {
        if let Ok(Some(storage)) = win.local_storage() {
            match slug {
                Some(s) => {
                    let _ = storage.set_item(STORAGE_KEY, s);
                }
                None => {
                    let _ = storage.remove_item(STORAGE_KEY);
                }
            }
        }
    }
}

fn role_pill(role: &str) -> &'static str {
    match role {
        "owner" => "pill pill-evergreen",
        "treasurer" => "pill pill-amber",
        _ => "pill pill-neutral",
    }
}

fn initial_of(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().next().unwrap_or(c).to_string())
        .unwrap_or_else(|| "—".to_string())
}

// ── Sidenav ─────────────────────────────────────────────────────────────
#[component]
pub fn Sidenav(active: String, children: Element) -> Element {
    let initial = read_stored();
    let mut selected_slug: Signal<Option<String>> = use_signal(|| initial);
    let mut ventures_sig: Signal<Option<Vec<Venture>>> = use_signal(|| None);
    let mut error_sig: Signal<Option<ApiError>> = use_signal(|| None);

    use_context_provider(|| VentureCtx {
        selected: selected_slug,
        ventures: ventures_sig,
        error: error_sig,
    });

    // Fetch + mirror into signals
    let ventures_res = use_resource(|| async move { fetch_ventures().await });
    use_effect(move || {
        match ventures_res() {
            None => { /* still loading */ }
            Some(Err(e)) => {
                error_sig.set(Some(e));
                ventures_sig.set(Some(Vec::new()));
            }
            Some(Ok(list)) => {
                error_sig.set(None);
                // Validate stored selection is still in the list
                let cur = selected_slug.peek().clone();
                let still_valid = match cur.as_deref() {
                    Some(s) => list.iter().any(|v| v.slug == s),
                    None => false,
                };
                if !still_valid {
                    let new_sel = list.first().map(|v| v.slug.clone());
                    selected_slug.set(new_sel.clone());
                    write_stored(new_sel.as_deref());
                }
                ventures_sig.set(Some(list));
            }
        }
    });

    rsx! {
        div {
            class: "min-h-screen bg-bone text-ink font-body antialiased flex",

            // ── Left rail ───────────────────────────────────────────────
            aside {
                class: "hidden md:flex w-[268px] shrink-0 flex-col border-r border-rule bg-paper sticky top-0 h-screen",

                // Brand
                a {
                    href: "/",
                    class: "px-6 py-5 flex items-center gap-3 border-b border-rule hover:bg-bone-soft transition-colors shrink-0",
                    span {
                        class: "text-[22px] text-evergreen leading-none",
                        style: "font-family: var(--font-tamil);",
                        "ஷ"
                    }
                    span {
                        class: "font-display text-[18px] font-semibold tracking-[-0.01em] text-ink",
                        "Sharam"
                    }
                }

                // ── Picker ─────────────────────────────────────────────
                div {
                    class: "px-3 pt-4 pb-4 border-b border-rule shrink-0",
                    SidenavPicker {}
                }

                // ── Venture-scoped nav (contextual) ────────────────────
                VentureNav { active: active.clone() }

                // Primary nav
                nav {
                    class: "px-3 py-4 flex-1 overflow-y-auto",

                    p { class: "eyebrow px-3 mb-2", "ACCOUNT" }
                    NavItem { href: "/profile",  label: "Profile",  icon: "◉", active: active == "profile" }
                    NavItem { href: "/settings", label: "Settings", icon: "⚙", active: active == "settings" }
                    button {
                        r#type: "button",
                        onclick: move |_| sign_out(),
                        class: "group w-full text-left flex items-center gap-3 px-3 py-2 rounded-md text-[13.5px] text-ink-soft hover:bg-negative-soft hover:text-negative transition-colors",
                        span { class: "w-5 inline-flex items-center justify-center text-[13px] text-ink-faint group-hover:text-negative", "↩" }
                        span { "Sign out" }
                    }
                }

                // Footer
                div {
                    class: "px-6 py-4 border-t border-rule text-[11px] text-ink-faint font-mono tracking-[0.14em] uppercase shrink-0",
                    "v0.1.0 · build 0001"
                }
            }

            // ── Main column ─────────────────────────────────────────────
            main {
                class: "flex-1 min-w-0",

                // Mobile top bar (sidenav hidden < md)
                div {
                    class: "md:hidden flex flex-col border-b border-rule bg-paper",
                    div {
                        class: "flex items-center justify-between px-4 py-3",
                        a {
                            href: "/",
                            class: "flex items-center gap-2",
                            span {
                                class: "text-[20px] text-evergreen leading-none",
                                style: "font-family: var(--font-tamil);",
                                "ஷ"
                            }
                            span { class: "font-display text-[16px] font-semibold", "Sharam" }
                        }
                        nav {
                            class: "flex items-center gap-3 text-[12.5px]",
                            a { href: "/dashboard", class: "text-ink-soft hover:text-evergreen", "Dashboard" }
                            a { href: "/profile", class: "text-ink-soft hover:text-evergreen", "Profile" }
                            button {
                                r#type: "button",
                                onclick: move |_| sign_out(),
                                class: "text-ink-soft hover:text-negative",
                                "Sign out"
                            }
                        }
                    }
                    div {
                        class: "px-3 pb-3 border-t border-rule-soft pt-3",
                        SidenavPicker {}
                    }
                }

                {children}
            }
        }
    }
}

// ── Picker ──────────────────────────────────────────────────────────────
#[component]
fn SidenavPicker() -> Element {
    let mut ctx = use_context::<VentureCtx>();
    let mut open = use_signal(|| false);

    let ventures_opt = (ctx.ventures)();
    let selected_now = (ctx.selected)();
    let err = (ctx.error)();

    rsx! {
        p { class: "eyebrow px-3 mb-2", "ACTIVE VENTURE" }

        {match (ventures_opt, err) {
            (None, _) => rsx! {
                div {
                    class: "mx-1 px-3 py-3 rounded-md bg-bone-soft border border-rule-soft text-[12px] text-ink-faint font-mono tracking-[0.06em]",
                    "Walking the directory…"
                }
            },
            (Some(_), Some(ApiError::NotSignedIn)) | (Some(_), Some(ApiError::Unauthorized)) => rsx! {
                a {
                    href: "/login",
                    class: "mx-1 px-3 py-3 rounded-md bg-bone-soft border border-rule-soft text-[12.5px] text-ink-soft font-mono tracking-[0.04em] flex items-center justify-between hover:border-evergreen/40 hover:text-evergreen transition-colors",
                    span { "Sign in" }
                    span { "→" }
                }
            },
            (Some(_), Some(_)) => rsx! {
                div {
                    class: "mx-1 px-3 py-3 rounded-md bg-negative-soft border border-negative/20 text-[12px] text-negative font-mono tracking-[0.04em]",
                    "Couldn't load ventures"
                }
            },
            (Some(list), None) if list.is_empty() => rsx! {
                a {
                    href: "/tenants/new",
                    class: "group mx-1 px-3 py-3 rounded-md bg-evergreen-soft hover:bg-evergreen text-evergreen hover:text-paper text-[12.5px] font-medium flex items-center justify-between transition-colors",
                    span { "Create your first venture" }
                    span { class: "opacity-60 group-hover:opacity-100", "→" }
                }
            },
            (Some(list), None) => {
                let active = selected_now
                    .as_ref()
                    .and_then(|s| list.iter().find(|v| &v.slug == s).cloned());
                let is_open = open();
                let active_for_trigger = active.clone();

                rsx! {
                    div {
                        class: "relative px-1",

                        // Click-out backdrop
                        if is_open {
                            div {
                                class: "fixed inset-0 z-30",
                                onclick: move |_| open.set(false),
                            }
                        }

                        // Trigger
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                let cur = open();
                                open.set(!cur);
                            },
                            class: "relative z-40 group w-full text-left card-flat hover:border-evergreen/40 transition-colors px-3 py-3 flex items-center gap-3",

                            span {
                                class: "shrink-0 h-9 w-9 rounded-md bg-evergreen-soft text-evergreen flex items-center justify-center font-display font-semibold text-[14px]",
                                {
                                    match active_for_trigger.as_ref() {
                                        Some(v) => initial_of(&v.display_name),
                                        None => "—".to_string(),
                                    }
                                }
                            }

                            div {
                                class: "min-w-0 flex-1",
                                p {
                                    class: "font-display text-[14px] font-semibold text-ink truncate group-hover:text-evergreen transition-colors",
                                    {
                                        match active_for_trigger.as_ref() {
                                            Some(v) => v.display_name.clone(),
                                            None => "Pick a venture".to_string(),
                                        }
                                    }
                                }
                                p {
                                    class: "mt-0.5 font-mono text-[10.5px] text-ink-faint truncate uppercase tracking-[0.1em]",
                                    {
                                        match active_for_trigger.as_ref() {
                                            Some(v) => format!("{} · ns={}", v.role, v.slug),
                                            None => format!("{} available", list.len()),
                                        }
                                    }
                                }
                            }

                            span {
                                class: if is_open {
                                    "shrink-0 text-evergreen text-[12px] transition-transform rotate-90"
                                } else {
                                    "shrink-0 text-ink-faint text-[12px] transition-transform group-hover:text-evergreen"
                                },
                                "›"
                            }
                        }

                        // Panel
                        if is_open {
                            div {
                                class: "absolute z-40 left-1 right-1 mt-2 card overflow-hidden",
                                style: "animation: rise 0.18s cubic-bezier(0.2,0.7,0.2,1) both;",

                                div {
                                    class: "px-3 py-2 border-b border-rule-soft flex items-center justify-between",
                                    p { class: "eyebrow", "SWITCH" }
                                    a {
                                        href: "/tenants/new",
                                        class: "text-[10.5px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40 font-mono uppercase tracking-[0.1em]",
                                        "+ NEW"
                                    }
                                }
                                ul {
                                    class: "max-h-[280px] overflow-y-auto",
                                    for v in list.iter() {
                                        SidenavPickerOption {
                                            venture: v.clone(),
                                            is_active: active.as_ref().map(|a| a.slug == v.slug).unwrap_or(false),
                                            on_pick: move |slug: String| {
                                                ctx.selected.set(Some(slug.clone()));
                                                write_stored(Some(&slug));
                                                open.set(false);
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }}
    }
}

#[component]
fn SidenavPickerOption(
    venture: Venture,
    is_active: bool,
    on_pick: EventHandler<String>,
) -> Element {
    let pill_cls = role_pill(&venture.role);
    let initial = initial_of(&venture.display_name);
    let slug = venture.slug.clone();
    let row_cls = if is_active {
        "w-full text-left px-3 py-2.5 bg-evergreen-soft/60 flex items-center gap-2.5 transition-colors"
    } else {
        "w-full text-left px-3 py-2.5 hover:bg-bone-soft flex items-center gap-2.5 transition-colors"
    };
    let badge_cls = if is_active {
        "shrink-0 h-7 w-7 rounded-md bg-evergreen text-paper flex items-center justify-center font-display font-semibold text-[12px]"
    } else {
        "shrink-0 h-7 w-7 rounded-md bg-bone-soft text-ink-soft flex items-center justify-center font-display font-semibold text-[12px]"
    };
    let name_cls = if is_active {
        "font-display text-[13px] font-semibold text-evergreen truncate"
    } else {
        "font-display text-[13px] font-semibold text-ink truncate"
    };

    rsx! {
        li {
            class: "border-b border-rule-soft last:border-b-0",
            button {
                r#type: "button",
                onclick: move |_| on_pick.call(slug.clone()),
                class: "{row_cls}",
                span { class: "{badge_cls}", "{initial}" }
                div {
                    class: "min-w-0 flex-1",
                    p { class: "{name_cls}", "{venture.display_name}" }
                    p {
                        class: "mt-0.5 font-mono text-[10px] text-ink-faint truncate uppercase tracking-[0.1em]",
                        "ns={venture.slug}"
                    }
                }
                span { class: "{pill_cls}", "{venture.role}" }
                if is_active {
                    span { class: "ml-1 text-evergreen text-[12px]", "✓" }
                }
            }
        }
    }
}

// ── Venture-scoped nav ──────────────────────────────────────────────────
#[component]
fn VentureNav(active: String) -> Element {
    let ctx = use_context::<VentureCtx>();
    let active_v: Option<Venture> = (ctx.ventures)()
        .and_then(|list| (ctx.selected)().and_then(|s| list.iter().find(|v| v.slug == s).cloned()));

    let is_admin = active_v
        .as_ref()
        .map(|v| v.role == "owner" || v.role == "treasurer")
        .unwrap_or(false);

    rsx! {
        div {
            class: "px-3 pt-4 pb-4 border-b border-rule shrink-0",
            p { class: "eyebrow px-3 mb-2", "VENTURE" }

            NavItem {
                href: "/dashboard".to_string(),
                label: "Overview".to_string(),
                icon: "▣".to_string(),
                active: active == "venture-overview",
            }

            if let Some(av) = active_v {
                NavItem {
                    href: format!("/ventures/{}", av.slug),
                    label: if is_admin { "Manage".to_string() } else { "Open ledger".to_string() },
                    icon: "▤".to_string(),
                    active: active == "venture-manage",
                }
                if is_admin {
                    NavItem {
                        href: format!("/ventures/{}/invites", av.slug),
                        label: "Invites".to_string(),
                        icon: "✉".to_string(),
                        active: active == "venture-invites",
                    }
                }
            }
        }
    }
}

#[component]
fn NavItem(href: String, label: String, icon: String, active: bool) -> Element {
    let base = "group flex items-center gap-3 px-3 py-2 rounded-md text-[13.5px] transition-colors";
    let cls = if active {
        format!("{base} bg-evergreen-soft text-evergreen font-medium")
    } else {
        format!("{base} text-ink-soft hover:bg-bone-soft hover:text-ink")
    };
    let icon_cls = if active {
        "w-5 inline-flex items-center justify-center text-[13px] text-evergreen"
    } else {
        "w-5 inline-flex items-center justify-center text-[13px] text-ink-faint group-hover:text-ink-soft"
    };
    rsx! {
        a {
            href: "{href}",
            class: "{cls}",
            span { class: "{icon_cls}", "{icon}" }
            span { "{label}" }
        }
    }
}
