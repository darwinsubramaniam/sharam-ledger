use dioxus::prelude::*;

mod api;
mod pages;
use pages::admin::AdminInvites;
use pages::create_tenant::CreateTenant;
use pages::dashboard::Dashboard;
use pages::login::Login;
use pages::profile::ProfilePage;
use pages::settings::Settings;
use pages::venture::VenturePage;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[route("/")]
    Home {},
    #[route("/login")]
    Login {},
    #[route("/dashboard")]
    Dashboard {},
    #[route("/ventures/:slug")]
    VenturePage { slug: String },
    #[route("/tenants/new")]
    CreateTenant {},
    #[route("/profile")]
    ProfilePage {},
    #[route("/settings")]
    Settings {},
    #[route("/ventures/:slug/invites")]
    AdminInvites { slug: String },
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

const GOOGLE_CLIENT_ID: &str = match option_env!("SHARAM_GOOGLE_CLIENT_ID") {
    Some(id) => id,
    None => "MISSING_GOOGLE_CLIENT_ID",
};

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: MAIN_CSS }
        document::Stylesheet { href: TAILWIND_CSS }
        document::Meta {
            name: "sharam-google-client-id",
            content: GOOGLE_CLIENT_ID,
        }
        Router::<Route> {}
    }
}

#[component]
fn Home() -> Element {
    rsx! {
        div {
            class: "min-h-screen bg-bone text-ink relative overflow-hidden paper-grain",

            // Soft warm gradient wash
            div {
                class: "absolute inset-0 pointer-events-none",
                style: "background:\
                    radial-gradient(60% 40% at 50% 0%, rgba(31,77,61,0.06), transparent 70%),\
                    radial-gradient(40% 30% at 50% 100%, rgba(184,112,61,0.05), transparent 70%);",
            }

            // Top bar
            header {
                class: "relative z-10 px-8 lg:px-14 py-6 flex items-center justify-between rise",
                div {
                    class: "flex items-center gap-3",
                    span {
                        class: "text-[22px] text-evergreen leading-none",
                        style: "font-family: var(--font-tamil);",
                        "ஷ"
                    }
                    span { class: "font-display text-[18px] font-semibold tracking-[-0.01em] text-ink", "Sharam" }
                }
                nav {
                    class: "flex items-center gap-7",
                    Link {
                        to: Route::Dashboard {},
                        class: "font-body text-[14px] text-ink-soft hover:text-evergreen transition-colors",
                        "Dashboard"
                    }
                    Link {
                        to: Route::Login {},
                        class: "inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep text-paper font-body text-[13px] font-medium tracking-wide px-4 py-2 rounded-md transition-colors",
                        "Sign in"
                        span { class: "text-[16px] leading-none -mt-0.5", "→" }
                    }
                }
            }

            // Hero
            main {
                class: "relative z-[1] px-8 lg:px-14 pt-20 pb-32 max-w-5xl mx-auto rise",
                style: "animation-delay: 0.1s",

                p { class: "eyebrow mb-6", "EST. 2026 · COOPERATIVE LEDGER · INVITE-ONLY" }

                h1 {
                    class: "display text-[clamp(3rem,7vw,5.75rem)] font-light leading-[1.02] text-ink max-w-4xl",
                    "A patient ledger for the capital you commit "
                    span {
                        class: "italic font-medium text-evergreen",
                        style: "font-variation-settings: 'opsz' 144, 'SOFT' 100;",
                        "together"
                    }
                    "."
                }

                p {
                    class: "mt-8 text-[18px] leading-[1.6] text-ink-soft max-w-2xl font-light",
                    "Track every member's monthly contribution, every receipt of payment, and every venture the pool deploys into — capital in, returns or losses out. Periods close. Ledgers harden."
                }

                div {
                    class: "mt-12 flex items-center gap-6",
                    Link {
                        to: Route::Dashboard {},
                        class: "inline-flex items-center gap-3 bg-evergreen hover:bg-evergreen-deep text-paper font-body text-[15px] font-medium tracking-wide px-6 py-3.5 rounded-md transition-colors",
                        "Open the ledger"
                        span { class: "text-[18px] leading-none", "→" }
                    }
                    span { class: "text-[13px] text-ink-faint",
                        "Need access? "
                        a {
                            href: "mailto:invite@sharam.coop",
                            class: "text-copper hover:text-evergreen border-b border-copper/40 hover:border-evergreen transition-colors",
                            "request an invite"
                        }
                    }
                }

                // Trust strip
                div {
                    class: "mt-24 pt-10 border-t border-rule grid grid-cols-1 sm:grid-cols-3 gap-8",
                    TrustItem {
                        eyebrow: "Period-locked",
                        body: "Closed months become permanent record. No retroactive edits, no quiet rewrites.",
                    }
                    TrustItem {
                        eyebrow: "Receipt-proven",
                        body: "Every contribution carries its own evidence. Stored privately, retrievable by member or treasurer.",
                    }
                    TrustItem {
                        eyebrow: "Member-scoped",
                        body: "Each venture is its own isolated ledger. Roles control what is seen and what is signed.",
                    }
                }
            }

            // Footer
            footer {
                class: "absolute bottom-0 left-0 right-0 px-8 lg:px-14 py-6 flex items-center justify-between text-[12px] text-ink-faint",
                span { "© 2026 Sharam Cooperative" }
                span { class: "font-mono tracking-[0.18em] uppercase text-[10.5px]", "v0.1.0 · Build 0001" }
            }
        }
    }
}

#[component]
fn TrustItem(eyebrow: String, body: String) -> Element {
    rsx! {
        div {
            p { class: "eyebrow mb-3", "{eyebrow}" }
            p { class: "text-[14.5px] leading-[1.55] text-ink-soft font-light", "{body}" }
        }
    }
}
