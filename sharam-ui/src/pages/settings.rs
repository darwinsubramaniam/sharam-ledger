use dioxus::prelude::*;

use crate::pages::sidenav::Sidenav;

#[component]
pub fn Settings() -> Element {
    rsx! {
        Sidenav { active: "settings".to_string(),
            section {
                class: "px-6 lg:px-12 pt-10 pb-6 max-w-[900px] rise",
                p { class: "eyebrow mb-4", "PREFERENCES" }
                h1 {
                    class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.1] text-ink",
                    "Settings"
                }
                p {
                    class: "mt-3 text-[15px] text-ink-soft font-light max-w-xl",
                    "Sharam-wide preferences live here. Per-venture settings (timezone, currency) are managed inside each venture's admin area."
                }
            }

            section {
                class: "px-6 lg:px-12 pb-16 max-w-[900px] rise",
                style: "animation-delay: 0.05s",

                div {
                    class: "card p-7",

                    SettingRow {
                        label: "Theme".to_string(),
                        description: "Bone (light) — high-contrast paper aesthetic.".to_string(),
                        control: rsx! {
                            span { class: "pill pill-evergreen", "Bone · default" }
                        },
                    }

                    div { class: "h-px my-5 bg-rule" }

                    SettingRow {
                        label: "Display timezone".to_string(),
                        description: "Used when no per-venture timezone applies. Your machine's timezone is used today.".to_string(),
                        control: rsx! {
                            span { class: "font-mono text-[12.5px] text-ink", "system" }
                        },
                    }

                    div { class: "h-px my-5 bg-rule" }

                    SettingRow {
                        label: "Notifications".to_string(),
                        description: "Email alerts on period close, treasurer review, and contribution rejections.".to_string(),
                        control: rsx! {
                            span { class: "pill pill-neutral", "Coming soon" }
                        },
                    }

                    div { class: "h-px my-5 bg-rule" }

                    SettingRow {
                        label: "Data export".to_string(),
                        description: "Download a JSON snapshot of your ventures and contributions.".to_string(),
                        control: rsx! {
                            span { class: "pill pill-neutral", "Coming soon" }
                        },
                    }
                }

                p {
                    class: "mt-6 text-[12px] text-ink-faint leading-relaxed",
                    "Settings are not yet persisted server-side. This page renders the surfaces; we'll wire them as the API expands."
                }
            }
        }
    }
}

#[component]
fn SettingRow(label: String, description: String, control: Element) -> Element {
    rsx! {
        div {
            class: "flex items-start justify-between gap-6",
            div {
                class: "min-w-0",
                p { class: "font-display text-[15px] font-semibold text-ink", "{label}" }
                p { class: "mt-1 text-[13px] text-ink-soft leading-relaxed", "{description}" }
            }
            div { class: "shrink-0", {control} }
        }
    }
}
