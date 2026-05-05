use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use crate::api::{current_user, sign_out, ApiError, UserClaims};
use crate::pages::sidenav::Sidenav;

fn format_unix(ts: i64) -> String {
    if ts <= 0 {
        return "—".to_string();
    }
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn expiry_hint(exp: i64) -> Option<String> {
    if exp <= 0 {
        return None;
    }
    let now = Utc::now().timestamp();
    let delta = exp - now;
    if delta <= 0 {
        Some("expired".to_string())
    } else if delta < 60 {
        Some(format!("in {delta}s"))
    } else if delta < 3600 {
        Some(format!("in {}m", delta / 60))
    } else {
        Some(format!("in {}h {}m", delta / 3600, (delta % 3600) / 60))
    }
}

#[component]
pub fn ProfilePage() -> Element {
    let view = current_user();
    let on_sign_out = move |_| sign_out();

    rsx! {
        Sidenav { active: "profile".to_string(),
            section {
                class: "px-6 lg:px-12 pt-10 pb-6 max-w-[900px] rise",
                p { class: "eyebrow mb-4", "ACCOUNT" }
                h1 {
                    class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.1] text-ink",
                    "Profile"
                }
                p {
                    class: "mt-3 text-[15px] text-ink-soft font-light max-w-xl",
                    "Identity from your Google account. Sharam never sees your password."
                }
            }

            section {
                class: "px-6 lg:px-12 pb-16 max-w-[900px] rise",
                style: "animation-delay: 0.05s",
                {match view {
                    Err(ApiError::NotSignedIn) | Err(ApiError::Unauthorized) => rsx! {
                        div {
                            class: "card p-8",
                            p { class: "eyebrow mb-2", "NOT SIGNED IN" }
                            p { class: "text-[14px] text-ink-soft mb-5",
                                "Sign in with Google to view your profile."
                            }
                            a {
                                href: "/login",
                                class: "inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper text-[14px] font-medium px-5 py-2.5 rounded-md transition-colors",
                                "Sign in"
                                span { class: "text-[16px] leading-none", "→" }
                            }
                        }
                    },
                    Err(e) => {
                        let msg = e.to_string();
                        rsx! {
                            div {
                                class: "card p-8",
                                p { class: "eyebrow !text-negative mb-2", "ERROR" }
                                p { class: "text-[14px] text-ink-soft", "{msg}" }
                            }
                        }
                    },
                    Ok(p) => rsx! { ProfileCard { profile: p, on_sign_out } },
                }}
            }
        }
    }
}

#[component]
fn ProfileCard(profile: UserClaims, on_sign_out: EventHandler<MouseEvent>) -> Element {
    let initials: String = profile
        .name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let issued = format_unix(profile.iat);
    let expires_at = format_unix(profile.exp);
    let expires_hint = expiry_hint(profile.exp);
    let expires_value = match expires_hint.as_deref() {
        Some("expired") => format!("{expires_at} (expired)"),
        Some(rel) => format!("{expires_at} ({rel})"),
        None => expires_at,
    };
    let expired = matches!(expires_hint.as_deref(), Some("expired"));

    rsx! {
        div {
            class: "card p-7",

            div {
                class: "flex items-start gap-5",
                if !profile.picture.is_empty() {
                    img {
                        src: "{profile.picture}",
                        alt: "{profile.name}",
                        class: "w-[72px] h-[72px] rounded-full border border-rule object-cover",
                    }
                } else {
                    div {
                        class: "w-[72px] h-[72px] rounded-full bg-evergreen-soft text-evergreen flex items-center justify-center font-display text-[24px] font-semibold border border-rule",
                        "{initials}"
                    }
                }
                div {
                    class: "min-w-0 flex-1",
                    h2 {
                        class: "font-display text-[22px] font-semibold text-ink truncate",
                        "{profile.name}"
                    }
                    p {
                        class: "mt-1 text-[14px] text-ink-soft truncate",
                        "{profile.email}"
                    }
                    p {
                        class: "mt-2 font-mono text-[11.5px] text-ink-faint break-all",
                        "google_sub: {profile.sub}"
                    }
                }
            }

            div {
                class: "mt-7 pt-5 border-t border-rule grid grid-cols-1 sm:grid-cols-2 gap-4 text-[12.5px]",
                InfoRow { label: "Email".to_string(), value: profile.email.clone(), negative: false }
                InfoRow { label: "Display name".to_string(), value: profile.name.clone(), negative: false }
                InfoRow { label: "Token issued".to_string(), value: issued, negative: false }
                InfoRow { label: "Token expires".to_string(), value: expires_value, negative: expired }
            }
        }

        div {
            class: "mt-5 flex items-center gap-3",
            button {
                r#type: "button",
                onclick: move |evt| on_sign_out.call(evt),
                class: "inline-flex items-center gap-2 bg-paper border border-rule hover:border-negative text-ink hover:text-negative font-medium text-[13.5px] px-4 py-2 rounded-md transition-colors",
                "Sign out"
            }
            p {
                class: "text-[12px] text-ink-faint",
                "Clears the local token and returns you to the sign-in page."
            }
        }
    }
}

#[component]
fn InfoRow(label: String, value: String, negative: bool) -> Element {
    let value_cls = if negative {
        "text-negative text-[13.5px] break-all font-medium"
    } else {
        "text-ink text-[13.5px] break-all"
    };
    rsx! {
        div {
            p { class: "eyebrow mb-1", "{label}" }
            p { class: "{value_cls}", "{value}" }
        }
    }
}
