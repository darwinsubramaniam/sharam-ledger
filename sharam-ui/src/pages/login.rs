use dioxus::prelude::*;
use serde_json::Value as JsonValue;

use crate::api::{login_with_password, register_with_password, store_token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Email,
    Google,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    SignIn,
    Register,
}

#[derive(Debug, Clone, PartialEq)]
enum GoogleState {
    Idle,
    Pending,
    Verifying,
    Error(String),
}

impl GoogleState {
    fn busy(&self) -> bool {
        matches!(self, GoogleState::Pending | GoogleState::Verifying)
    }

    fn label(&self) -> &'static str {
        match self {
            GoogleState::Pending => "Awaiting Google…",
            GoogleState::Verifying => "Verifying signature…",
            _ => "Continue with Google",
        }
    }
}

/// Google Identity Services callback. The gateway response now carries a
/// Sharam-issued session JWT in `token`; we store that (not the raw Google
/// credential) so the rest of the app sees a single token type regardless
/// of sign-in method.
const SIGNIN_JS: &str = r#"
return new Promise((resolve) => {
  try {
    const meta = document.querySelector('meta[name="sharam-google-client-id"]');
    const clientId = meta && meta.content;
    if (!clientId) {
      return resolve({ ok: false, message: "Sign-in config not loaded yet — try again in a moment" });
    }
    if (!window.google || !window.google.accounts || !window.google.accounts.id) {
      return resolve({ ok: false, message: "Google sign-in script failed to load" });
    }
    let done = false;
    const finish = (v) => { if (!done) { done = true; resolve(v); } };
    google.accounts.id.initialize({
      client_id: clientId,
      callback: async (resp) => {
        try {
          const r = await fetch("/api/auth/google", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ credential: resp.credential }),
          });
          const body = await r.json().catch(() => null);
          if (r.ok && body && body.token) {
            try { localStorage.setItem("sharam_id_token", body.token); } catch (_) {}
            window.location.assign("/dashboard");
            finish({ ok: true });
          } else {
            const msg = (body && body.error) || ("Server rejected sign-in (" + r.status + ")");
            finish({ ok: false, message: msg });
          }
        } catch (e) {
          finish({ ok: false, message: "Network error: " + String(e) });
        }
      },
      ux_mode: "popup",
      auto_select: false,
      itp_support: true,
    });
    google.accounts.id.prompt((n) => {
      try {
        if ((n.isNotDisplayed && n.isNotDisplayed()) || (n.isSkippedMoment && n.isSkippedMoment())) {
          finish({ ok: false, message: "Sign-in dismissed" });
        }
      } catch (_) {}
    });
  } catch (e) {
    resolve({ ok: false, message: "Sign-in error: " + String(e) });
  }
});
"#;

#[component]
pub fn Login() -> Element {
    let mut tab = use_signal(|| Tab::Email);

    rsx! {
        div {
            class: "min-h-screen bg-bone text-ink font-body antialiased relative overflow-hidden",

            div {
                class: "absolute inset-0 pointer-events-none",
                style: "background:\
                    radial-gradient(50% 40% at 0% 0%, rgba(31,77,61,0.06), transparent 70%),\
                    radial-gradient(40% 50% at 100% 100%, rgba(184,112,61,0.05), transparent 70%);",
            }

            div {
                class: "grid lg:grid-cols-[1.05fr_1fr] min-h-screen relative",

                // ── LEFT: editorial brand statement ─────────────────────────
                section {
                    class: "relative px-5 pt-8 pb-12 sm:px-8 sm:pt-10 sm:pb-16 lg:px-16 lg:pt-14 flex flex-col justify-between rise",

                    div {
                        class: "flex items-center gap-3",
                        span {
                            class: "text-[22px] text-evergreen leading-none",
                            style: "font-family: var(--font-tamil);",
                            "ஷ"
                        }
                        span { class: "font-display text-[18px] font-semibold tracking-[-0.01em]", "Sharam" }
                    }

                    div {
                        class: "max-w-xl mt-24 lg:mt-0",

                        p { class: "eyebrow mb-6", "WELCOME · STEP 01 · IDENTITY" }

                        h1 {
                            class: "display text-[clamp(2.5rem,5.5vw,4.25rem)] font-light leading-[1.05] text-ink",
                            "Capital you commit. "
                            br {}
                            "Returns you can "
                            span {
                                class: "italic font-medium text-evergreen",
                                style: "font-variation-settings: 'opsz' 144, 'SOFT' 100;",
                                "verify"
                            }
                            "."
                        }

                        p {
                            class: "mt-8 text-[16px] leading-[1.65] text-ink-soft font-light",
                            "Sharam tracks every member's monthly stake, every receipt of payment, and every venture the pool deploys into. Periods close. Ledgers harden. Decisions become record."
                        }

                        div {
                            class: "mt-10 grid grid-cols-3 gap-6 max-w-md",
                            FactStat { eyebrow: "Members", value: "Invite-only" }
                            FactStat { eyebrow: "Periods", value: "Auto-locked" }
                            FactStat { eyebrow: "Receipts", value: "Verifiable" }
                        }
                    }

                    div {
                        class: "mt-16 lg:mt-0 text-[12px] text-ink-faint",
                        "© 2026 Sharam Cooperative · "
                        span { class: "font-mono tracking-[0.18em] uppercase text-[10.5px]", "v0.1.0" }
                    }
                }

                // ── RIGHT: auth card ────────────────────────────────────────
                section {
                    class: "relative flex items-center justify-center px-4 pb-12 pt-2 sm:px-6 sm:pb-16 sm:pt-4 lg:px-12 lg:py-0 rise",
                    style: "animation-delay: 0.1s",

                    div {
                        class: "w-full max-w-md",

                        div {
                            class: "card px-5 py-8 sm:px-8 sm:py-10 lg:px-10 lg:py-11",

                            p { class: "eyebrow", "AUTHENTICATE" }
                            h2 {
                                class: "display text-[2rem] font-semibold leading-[1.1] text-ink mt-3",
                                "Sign in to continue"
                            }
                            p {
                                class: "mt-3 text-[14.5px] text-ink-soft font-light leading-[1.55]",
                                "Use your email and password, or sign in with Google. Access to a venture still depends on an invite."
                            }

                            // Method tabs
                            div {
                                class: "mt-8 grid grid-cols-2 gap-2 p-1 rounded-md bg-bone-soft border border-rule",
                                TabButton {
                                    active: tab() == Tab::Email,
                                    onclick: move |_| tab.set(Tab::Email),
                                    label: "Email"
                                }
                                TabButton {
                                    active: tab() == Tab::Google,
                                    onclick: move |_| tab.set(Tab::Google),
                                    label: "Google"
                                }
                            }

                            div {
                                class: "mt-8",
                                {match tab() {
                                    Tab::Email => rsx! { EmailPanel {} },
                                    Tab::Google => rsx! { GooglePanel {} },
                                }}
                            }

                            div {
                                class: "mt-9 pt-6 border-t border-rule",
                                p { class: "text-[12.5px] text-ink-faint leading-relaxed",
                                    "Authentication binds your identity to the venture ledger. Period entries become permanent record at close."
                                }
                            }
                        }

                        div {
                            class: "mt-6 text-center",
                            p { class: "text-[13px] text-ink-faint",
                                "Need access to a venture? "
                                a {
                                    href: "mailto:invite@sharam.coop",
                                    class: "text-copper hover:text-evergreen border-b border-copper/40 hover:border-evergreen transition-colors",
                                    "Contact a treasurer"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TabButton(active: bool, onclick: EventHandler<MouseEvent>, label: String) -> Element {
    let cls = if active {
        "text-[13px] font-medium px-3 py-2 rounded bg-paper text-ink shadow-sm border border-rule"
    } else {
        "text-[13px] px-3 py-2 rounded text-ink-soft hover:text-ink transition-colors"
    };
    rsx! {
        button {
            r#type: "button",
            class: "{cls}",
            onclick: move |e| onclick.call(e),
            "{label}"
        }
    }
}

#[component]
fn EmailPanel() -> Element {
    let mut mode = use_signal(|| Mode::SignIn);
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal::<Option<String>>(|| None);

    let mode_now = mode();
    let cta_label = match (busy(), mode_now) {
        (true, Mode::SignIn) => "Signing in…",
        (true, Mode::Register) => "Creating account…",
        (false, Mode::SignIn) => "Sign in",
        (false, Mode::Register) => "Create account",
    };
    let toggle_label = match mode_now {
        Mode::SignIn => "Don't have an account? Create one",
        Mode::Register => "Already have an account? Sign in",
    };
    let pwd_hint = match mode_now {
        Mode::SignIn => "Enter your password",
        Mode::Register => "At least 8 characters",
    };

    rsx! {
        form {
            class: "space-y-4",
            onsubmit: move |ev| async move {
                ev.prevent_default();
                if busy() {
                    return;
                }
                error.set(None);
                let e = email().trim().to_string();
                let p = password();
                if e.is_empty() || p.is_empty() {
                    error.set(Some("Email and password are required.".into()));
                    return;
                }
                busy.set(true);
                let result = match mode() {
                    Mode::SignIn => login_with_password(&e, &p).await,
                    Mode::Register => {
                        let dn = display_name();
                        let dn_opt = if dn.trim().is_empty() {
                            None
                        } else {
                            Some(dn.trim())
                        };
                        register_with_password(&e, &p, dn_opt).await
                    }
                };
                match result {
                    Ok(success) => {
                        if let Err(store_err) = store_token(&success.token) {
                            error.set(Some(store_err.to_string()));
                            busy.set(false);
                            return;
                        }
                        if let Some(window) = web_sys::window() {
                            let _ = window.location().set_href("/dashboard");
                        }
                    }
                    Err(api_err) => {
                        error.set(Some(api_err.to_string()));
                        busy.set(false);
                    }
                }
            },

            if mode_now == Mode::Register {
                FieldLabel { eyebrow: "DISPLAY NAME · OPTIONAL",
                    input {
                        r#type: "text",
                        value: "{display_name}",
                        oninput: move |e| display_name.set(e.value()),
                        placeholder: "How you appear to the venture",
                        autocomplete: "name",
                        class: "field-input",
                    }
                }
            }

            FieldLabel { eyebrow: "EMAIL",
                input {
                    r#type: "email",
                    value: "{email}",
                    oninput: move |e| email.set(e.value()),
                    placeholder: "you@example.com",
                    autocomplete: "email",
                    required: true,
                    class: "field-input",
                }
            }

            FieldLabel { eyebrow: "PASSWORD",
                input {
                    r#type: "password",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                    placeholder: "{pwd_hint}",
                    autocomplete: if mode_now == Mode::Register { "new-password" } else { "current-password" },
                    minlength: if mode_now == Mode::Register { "8" } else { "1" },
                    required: true,
                    class: "field-input",
                }
            }

            if let Some(msg) = error() {
                div {
                    class: "px-3.5 py-2.5 rounded-md bg-negative-soft border border-negative/15",
                    p { class: "text-[12.5px] text-negative leading-relaxed",
                        span { class: "font-mono tracking-[0.12em] uppercase text-[10.5px] mr-2", "ERROR" }
                        "{msg}"
                    }
                }
            }

            button {
                r#type: "submit",
                disabled: busy(),
                class: "w-full inline-flex items-center justify-center bg-evergreen hover:bg-evergreen-deep text-paper font-body text-[15px] font-medium px-5 py-3.5 rounded-md transition-colors disabled:opacity-50 disabled:cursor-progress",
                "{cta_label}"
            }

            button {
                r#type: "button",
                onclick: move |_| {
                    error.set(None);
                    mode.set(if mode() == Mode::SignIn { Mode::Register } else { Mode::SignIn });
                },
                class: "w-full text-[13px] text-ink-soft hover:text-evergreen transition-colors pt-1",
                "{toggle_label}"
            }
        }
    }
}

#[component]
fn FieldLabel(eyebrow: String, children: Element) -> Element {
    rsx! {
        label {
            class: "block",
            span { class: "eyebrow mb-2 block", "{eyebrow}" }
            {children}
        }
    }
}

#[component]
fn GooglePanel() -> Element {
    let mut state = use_signal(|| GoogleState::Idle);

    let onclick = move |_| {
        if state.read().busy() {
            return;
        }
        state.set(GoogleState::Pending);
        spawn(async move {
            let mut handle = document::eval(SIGNIN_JS);
            match handle.recv::<JsonValue>().await {
                Ok(v) => {
                    let ok = v.get("ok").and_then(JsonValue::as_bool).unwrap_or(false);
                    if ok {
                        state.set(GoogleState::Verifying);
                    } else {
                        let msg = v
                            .get("message")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("Sign-in failed")
                            .to_string();
                        state.set(GoogleState::Error(msg));
                    }
                }
                Err(_) => state.set(GoogleState::Error(
                    "Could not reach Google sign-in".to_string(),
                )),
            }
        });
    };

    let current = state.read().clone();
    let busy = current.busy();
    let label = current.label();
    let error_msg = match &current {
        GoogleState::Error(m) => Some(m.clone()),
        _ => None,
    };

    rsx! {
        div {
            class: "space-y-5",

            p {
                class: "text-[13.5px] text-ink-soft leading-[1.55]",
                "We'll verify your Google identity and mint a Sharam session token. No password required."
            }

            button {
                r#type: "button",
                disabled: busy,
                onclick,
                class: "group w-full inline-flex items-center justify-center gap-3 bg-paper border border-rule hover:border-evergreen hover:bg-bone-soft text-ink font-body text-[15px] font-medium px-5 py-3.5 rounded-md transition-all disabled:opacity-50 disabled:cursor-progress",
                svg {
                    view_box: "0 0 24 24",
                    class: "w-[18px] h-[18px] block",
                    path { fill: "#4285F4", d: "M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" }
                    path { fill: "#34A853", d: "M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.99.66-2.25 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" }
                    path { fill: "#FBBC05", d: "M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" }
                    path { fill: "#EA4335", d: "M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" }
                }
                span { "{label}" }
            }

            if let Some(msg) = error_msg {
                div {
                    class: "px-3.5 py-2.5 rounded-md bg-negative-soft border border-negative/15",
                    p { class: "text-[12.5px] text-negative leading-relaxed",
                        span { class: "font-mono tracking-[0.12em] uppercase text-[10.5px] mr-2", "ERROR" }
                        "{msg}"
                    }
                }
            }
        }
    }
}

#[component]
fn FactStat(eyebrow: String, value: String) -> Element {
    rsx! {
        div {
            p { class: "eyebrow mb-1.5", "{eyebrow}" }
            p { class: "font-display text-[15px] font-medium text-ink", "{value}" }
        }
    }
}
