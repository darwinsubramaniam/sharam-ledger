use dioxus::prelude::*;
use dioxus_primitives::toast::{consume_toast, ToastOptions};
use serde::{Deserialize, Serialize};

use crate::api::{authed, into_api_error, ApiError};
use crate::pages::sidenav::Sidenav;

// ─── Wire shapes ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Owner,
    Treasurer,
    Member,
}

impl Role {
    fn wire(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Treasurer => "treasurer",
            Role::Member => "member",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Role::Owner => "Admin",
            Role::Treasurer => "Treasurer",
            Role::Member => "Member",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Role::Owner => "Full control. Manage members, roles, and venture settings.",
            Role::Treasurer => "Verify contributions, manage period locks, post receipts.",
            Role::Member => "Submit contributions and view ledger history.",
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct InviteView {
    id: String,
    #[allow(dead_code)]
    tenant_slug: String,
    email: String,
    role: String,
    status: String,
    created_at: String,
    #[allow(dead_code)]
    accepted_at: Option<String>,
    #[allow(dead_code)]
    revoked_at: Option<String>,
}

#[derive(Deserialize)]
struct InvitesResponse {
    invites: Vec<InviteView>,
}

#[derive(Deserialize, Clone)]
struct VentureRow {
    slug: String,
    display_name: String,
    role: String,
}

#[derive(Deserialize)]
struct VenturesResponse {
    ventures: Vec<VentureRow>,
}

#[derive(Serialize)]
struct CreateInviteRequest {
    email: String,
    role: &'static str,
}

// ─── API ───────────────────────────────────────────────────────────────────

/// Returns the venture row if the caller is an owner of `slug`. `Ok(None)`
/// means "no access / not an owner" so the page can render a forbidden
/// message instead of a generic error.
async fn fetch_owner_context(slug: &str) -> Result<Option<VentureRow>, ApiError> {
    let resp = authed(reqwest::Method::GET, "/api/me/ventures")?
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: VenturesResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))?;
    Ok(body
        .ventures
        .into_iter()
        .find(|v| v.slug == slug && v.role == "owner"))
}

async fn fetch_invites(slug: &str) -> Result<Vec<InviteView>, ApiError> {
    let resp = authed(
        reqwest::Method::GET,
        &format!("/api/tenants/{slug}/invites"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    let body: InvitesResponse = resp
        .json()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))?;
    Ok(body.invites)
}

async fn post_invite(slug: &str, req: CreateInviteRequest) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::POST,
        &format!("/api/tenants/{slug}/invites"),
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

async fn revoke_invite(slug: &str, key: &str) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::DELETE,
        &format!("/api/tenants/{slug}/invites/{key}"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    Ok(())
}

async fn delete_invite_permanent(slug: &str, key: &str) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::DELETE,
        &format!("/api/tenants/{slug}/invites/{key}/permanent"),
    )?
    .send()
    .await
    .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    Ok(())
}

// ─── Validation & helpers ──────────────────────────────────────────────────

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

fn pill_class(status: &str) -> &'static str {
    match status {
        "pending" => "pill pill-amber",
        "accepted" => "pill pill-positive",
        "revoked" => "pill pill-negative",
        _ => "pill pill-neutral",
    }
}

fn role_label(role: &str) -> &'static str {
    match role {
        "owner" => "Admin",
        "treasurer" => "Treasurer",
        "member" => "Member",
        _ => "—",
    }
}

// ─── Page ──────────────────────────────────────────────────────────────────

#[component]
pub fn AdminInvites(slug: String) -> Element {
    let slug_for_load = slug.clone();
    let owner_ctx = use_resource(move || {
        let s = slug_for_load.clone();
        async move { fetch_owner_context(&s).await }
    });

    rsx! {
        Sidenav { active: "venture-invites".to_string(),
            {match owner_ctx() {
                None => rsx! { LoadingShell {} },
                Some(Err(ApiError::NotSignedIn)) | Some(Err(ApiError::Unauthorized)) => {
                    rsx! { SignInPrompt {} }
                }
                Some(Err(e)) => {
                    let msg = e.to_string();
                    rsx! { ErrorPanel { message: msg } }
                }
                Some(Ok(None)) => rsx! { ForbiddenPanel { slug: slug.clone() } },
                Some(Ok(Some(v))) => rsx! {
                    Body { slug: slug.clone(), display_name: v.display_name.clone() }
                },
            }}
        }
    }
}

#[component]
fn Body(slug: String, display_name: String) -> Element {
    let slug_for_invites = slug.clone();
    let mut invites = use_resource(move || {
        let s = slug_for_invites.clone();
        async move { fetch_invites(&s).await }
    });

    let mut email = use_signal(String::new);
    let mut role = use_signal(|| Role::Member);
    let mut form_error: Signal<Option<String>> = use_signal(|| None);
    let mut submitting = use_signal(|| false);

    let submit = {
        let slug = slug.clone();
        move |_| {
            let slug = slug.clone();
            async move {
                let e = email.read().trim().to_string();
                if !valid_email(&e) {
                    form_error.set(Some(
                        "Enter a valid email address (e.g. name@domain.com).".into(),
                    ));
                    return;
                }
                form_error.set(None);
                submitting.set(true);
                let req = CreateInviteRequest {
                    email: e.clone(),
                    role: role().wire(),
                };
                match post_invite(&slug, req).await {
                    Ok(()) => {
                        let role_label = role().label().to_string();
                        email.set(String::new());
                        consume_toast().success(
                            format!("Invite sent to {e}"),
                            ToastOptions::new().description(format!("Joining as {role_label}.")),
                        );
                        invites.restart();
                    }
                    Err(err) => form_error.set(Some(err.to_string())),
                }
                submitting.set(false);
            }
        }
    };

    rsx! {
        // Breadcrumb
        div {
            class: "px-4 sm:px-6 lg:px-12 pt-7 pb-3 flex items-center gap-3 text-[12px] text-ink-faint font-mono tracking-[0.12em] uppercase rise",
            a { href: "/dashboard", class: "hover:text-evergreen transition-colors", "Dashboard" }
            span { "›" }
            a { href: "/ventures/{slug}", class: "hover:text-evergreen transition-colors", "{slug}" }
            span { "›" }
            span { class: "text-ink-soft", "Invites" }
        }

        // Hero
        section {
            class: "px-4 sm:px-6 lg:px-12 pt-2 pb-8 max-w-[1140px] rise",
            style: "animation-delay: 0.04s",
            p { class: "eyebrow mb-3", "INVITES · {slug}" }
            h1 { class: "display text-[clamp(2rem,4.5vw,3rem)] font-light leading-[1.05] text-ink",
                "Invite people to {display_name}"
            }
            p { class: "mt-3 text-[14px] text-ink-soft max-w-2xl",
                "Only admins of this venture can send invites. Pick a role for the invitee — Member is the default and can submit contributions; Treasurer can verify them; Admin gets full control."
            }
        }

        section {
            class: "px-4 sm:px-6 lg:px-12 pb-12 max-w-[1140px] grid grid-cols-1 lg:grid-cols-12 gap-6 rise",
            style: "animation-delay: 0.10s",

            // Form
            div { class: "lg:col-span-5 card p-6",
                p { class: "eyebrow mb-5", "SEND AN INVITE" }

                label { class: "block text-[12.5px] font-medium text-ink-soft mb-2", "Email" }
                input {
                    r#type: "email",
                    autocomplete: "off",
                    spellcheck: "false",
                    placeholder: "name@domain.com",
                    value: "{email}",
                    oninput: move |e| {
                        email.set(e.value());
                        form_error.set(None);
                    },
                    class: "w-full bg-paper border border-rule focus:border-evergreen focus:ring-2 focus:ring-evergreen/15 outline-none rounded-md px-3.5 py-2.5 text-[14px] text-ink transition",
                }

                div { class: "mt-5",
                    label { class: "block text-[12.5px] font-medium text-ink-soft mb-2", "Role" }
                    div { class: "space-y-2",
                        for r_opt in [Role::Member, Role::Treasurer, Role::Owner] {
                            {
                                let selected = role() == r_opt;
                                let label = r_opt.label();
                                let desc = r_opt.description();
                                let cls = if selected {
                                    "w-full text-left p-3 rounded-md border-2 border-evergreen bg-evergreen-soft/40 transition-colors"
                                } else {
                                    "w-full text-left p-3 rounded-md border border-rule bg-paper hover:bg-bone-soft transition-colors"
                                };
                                let dot_cls = if selected {
                                    "mt-0.5 h-4 w-4 shrink-0 rounded-full border-2 border-evergreen flex items-center justify-center"
                                } else {
                                    "mt-0.5 h-4 w-4 shrink-0 rounded-full border-2 border-rule flex items-center justify-center"
                                };
                                rsx! {
                                    button {
                                        key: "{label}",
                                        r#type: "button",
                                        onclick: move |_| role.set(r_opt),
                                        class: "{cls}",
                                        div { class: "flex items-start gap-3",
                                            span { class: "{dot_cls}",
                                                if selected { span { class: "h-2 w-2 rounded-full bg-evergreen" } }
                                            }
                                            div {
                                                p { class: "text-[13.5px] font-medium text-ink", "{label}" }
                                                p { class: "mt-0.5 text-[12px] text-ink-faint leading-[1.5]", "{desc}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                button {
                    r#type: "button",
                    disabled: submitting(),
                    onclick: submit,
                    class: "mt-6 w-full inline-flex items-center justify-center gap-2 bg-evergreen hover:bg-evergreen-deep disabled:opacity-60 disabled:cursor-not-allowed text-paper font-medium text-[14px] px-5 py-2.5 rounded-md transition-colors",
                    if submitting() { "Sending…" } else { "Send invite" }
                }

                if let Some(msg) = form_error() {
                    div { class: "mt-4 px-3.5 py-2.5 rounded-md bg-negative-soft border border-negative/15",
                        p { class: "text-[12.5px] text-negative leading-relaxed", "{msg}" }
                    }
                }
            }

            // Invites table
            div { class: "lg:col-span-7",
                {match invites() {
                    None => rsx! {
                        div { class: "card p-10 text-center text-ink-soft text-[14px]",
                            "Loading invites…"
                        }
                    },
                    Some(Err(e)) => {
                        let msg = e.to_string();
                        rsx! {
                            div { class: "card p-6",
                                p { class: "eyebrow !text-negative mb-2", "ERROR" }
                                p { class: "text-[14px] text-ink-soft", "{msg}" }
                            }
                        }
                    },
                    Some(Ok(list)) if list.is_empty() => rsx! {
                        div { class: "card p-10 text-center",
                            p { class: "eyebrow mb-2", "NO INVITES YET" }
                            p { class: "text-[13.5px] text-ink-soft",
                                "Send the first invite using the form on the left."
                            }
                        }
                    },
                    Some(Ok(list)) => rsx! {
                        InvitesTable {
                            slug: slug.clone(),
                            list: list,
                            on_changed: move |_| invites.restart(),
                        }
                    },
                }}
            }
        }
    }
}

#[component]
fn InvitesTable(slug: String, list: Vec<InviteView>, on_changed: EventHandler<()>) -> Element {
    rsx! {
        div { class: "card overflow-hidden",
            div {
                class: "hidden sm:grid grid-cols-[2.4fr_1fr_1fr_180px] gap-4 px-5 py-3 bg-bone-soft border-b border-rule text-[11px] text-ink-faint font-mono uppercase tracking-[0.14em]",
                span { "Email" }
                span { "Role" }
                span { "Status" }
                span { class: "text-right", "" }
            }
            for inv in list.iter() {
                InviteRow {
                    key: "{inv.id}",
                    slug: slug.clone(),
                    invite: inv.clone(),
                    on_changed: on_changed,
                }
            }
        }
    }
}

#[component]
fn InviteRow(slug: String, invite: InviteView, on_changed: EventHandler<()>) -> Element {
    let mut busy = use_signal(|| false);
    let mut row_error: Signal<Option<String>> = use_signal(|| None);

    let pill_cls = pill_class(&invite.status);
    let role_lbl = role_label(&invite.role);
    let is_pending = invite.status == "pending";
    let is_revoked = invite.status == "revoked";
    let created = invite.created_at.get(..10).unwrap_or("").to_string();

    let on_revoke = {
        let slug = slug.clone();
        let key = invite.id.clone();
        move |_| {
            let slug = slug.clone();
            let key = key.clone();
            async move {
                busy.set(true);
                row_error.set(None);
                match revoke_invite(&slug, &key).await {
                    Ok(()) => on_changed.call(()),
                    Err(e) => row_error.set(Some(e.to_string())),
                }
                busy.set(false);
            }
        }
    };

    let on_delete = {
        let slug = slug.clone();
        let key = invite.id.clone();
        move |_| {
            let slug = slug.clone();
            let key = key.clone();
            async move {
                busy.set(true);
                row_error.set(None);
                match delete_invite_permanent(&slug, &key).await {
                    Ok(()) => on_changed.call(()),
                    Err(e) => row_error.set(Some(e.to_string())),
                }
                busy.set(false);
            }
        }
    };

    rsx! {
        div { class: "flex flex-col gap-2.5 sm:gap-4 sm:grid sm:grid-cols-[2.4fr_1fr_1fr_180px] sm:items-center px-4 sm:px-5 py-3 sm:py-3.5 border-b border-rule-soft last:border-b-0",
            div { class: "min-w-0",
                p { class: "text-[14px] text-ink font-medium break-all sm:truncate", "{invite.email}" }
                p { class: "text-[11.5px] text-ink-faint font-mono", "Sent {created}" }
            }
            div { class: "flex items-center gap-2 sm:hidden",
                span { class: "text-[12.5px] text-ink-soft", "{role_lbl}" }
                span { class: "text-ink-faint text-[11px]", "·" }
                span { class: "{pill_cls}", "{invite.status}" }
            }
            div { class: "hidden sm:block",
                p { class: "text-[13px] text-ink-soft", "{role_lbl}" }
            }
            div { class: "hidden sm:block",
                span { class: "{pill_cls}", "{invite.status}" }
            }
            div { class: "flex items-center justify-end gap-2 sm:text-right",
                if busy() {
                    span { class: "text-[12px] text-ink-faint font-mono", "…" }
                } else {
                    if is_pending {
                        button {
                            r#type: "button",
                            onclick: on_revoke,
                            class: "px-2.5 py-1 text-[12px] font-medium text-ink-soft hover:text-ink hover:bg-bone-soft rounded transition-colors",
                            "Revoke"
                        }
                    }
                    if is_pending || is_revoked {
                        button {
                            r#type: "button",
                            onclick: on_delete,
                            class: "px-2.5 py-1 text-[12px] font-medium text-negative hover:bg-negative-soft rounded transition-colors",
                            "Delete"
                        }
                    }
                }
            }
        }
        if let Some(msg) = row_error() {
            p { class: "px-5 py-2 text-[11.5px] text-negative font-mono border-b border-rule-soft",
                "{msg}"
            }
        }
    }
}

// ─── Shells ────────────────────────────────────────────────────────────────

#[component]
fn LoadingShell() -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            p { class: "eyebrow mb-3", "INVITES" }
            div { class: "card p-10 text-center text-ink-soft text-[14px]",
                "Verifying access…"
            }
        }
    }
}

#[component]
fn SignInPrompt() -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div { class: "card p-8",
                p { class: "eyebrow mb-2", "SESSION EXPIRED" }
                h2 { class: "font-display text-[20px] font-semibold text-ink", "You're not signed in" }
                p { class: "mt-2 text-[14px] text-ink-soft",
                    "Your sign-in session expired. Sign in again to manage invites."
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
fn ForbiddenPanel(slug: String) -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div { class: "card p-8",
                p { class: "eyebrow !text-amber mb-2", "ADMIN ONLY" }
                h2 { class: "font-display text-[20px] font-semibold text-ink",
                    "Only admins can invite to this venture"
                }
                p { class: "mt-2 text-[14px] text-ink-soft",
                    "You're not an admin (owner) of this venture, or you don't have access to it. Ask an existing admin to send the invite."
                }
                a {
                    href: "/ventures/{slug}",
                    class: "mt-5 inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40",
                    "← Back to venture"
                }
            }
        }
    }
}

#[component]
fn ErrorPanel(message: String) -> Element {
    rsx! {
        section { class: "px-4 sm:px-6 lg:px-12 pt-10 pb-16 max-w-[1140px]",
            div { class: "card p-8",
                p { class: "eyebrow !text-negative mb-2", "ERROR" }
                p { class: "text-[14px] text-ink-soft", "{message}" }
                a {
                    href: "/dashboard",
                    class: "mt-5 inline-flex items-center gap-2 text-[13px] text-evergreen hover:text-evergreen-deep border-b border-evergreen/40",
                    "← Back to dashboard"
                }
            }
        }
    }
}
