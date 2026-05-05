use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::{authed, into_api_error, ApiError};
use crate::pages::sidenav::Sidenav;

#[derive(Serialize)]
struct CreateTenantRequest {
    slug: String,
    display_name: String,
    timezone: String,
    currency: String,
    cadence: String,
    dues_amount_cents: i64,
}

#[derive(Serialize)]
struct CarryForwardRequest {
    from_date: String,
    to_date: String,
    amount_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

const CADENCES: &[(&str, &str)] = &[
    ("monthly", "Monthly"),
    ("weekly", "Weekly"),
    ("yearly", "Yearly"),
];

#[derive(Deserialize)]
struct CreateTenantResponse {
    slug: String,
    #[allow(dead_code)]
    display_name: String,
}

async fn submit_tenant(req: CreateTenantRequest) -> Result<CreateTenantResponse, ApiError> {
    let resp = authed(reqwest::Method::POST, "/api/tenants")?
        .json(&req)
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() {
        return Err(into_api_error(resp).await);
    }
    resp.json::<CreateTenantResponse>()
        .await
        .map_err(|e| ApiError::Other(format!("decode: {e}")))
}

async fn submit_carry_forward(slug: &str, req: CarryForwardRequest) -> Result<(), ApiError> {
    let resp = authed(
        reqwest::Method::POST,
        &format!("/api/tenants/{slug}/carry-forward"),
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

#[derive(Debug, Clone, PartialEq)]
struct SlugIssue(&'static str);

fn validate_slug(s: &str) -> Result<(), SlugIssue> {
    if s.is_empty() {
        return Err(SlugIssue("Slug is required."));
    }
    if s.len() < 3 {
        return Err(SlugIssue("Slug must be at least 3 characters."));
    }
    if s.len() > 41 {
        return Err(SlugIssue("Slug must be 41 characters or fewer."));
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return Err(SlugIssue("Slug must start with a lowercase letter."));
    }
    if !bytes
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
    {
        return Err(SlugIssue(
            "Slug may only contain lowercase letters, digits and underscore.",
        ));
    }
    Ok(())
}

/// Parse a major-unit money string ("25", "25.50") into integer cents.
/// Rejects negative values and more than 2 fractional digits.
fn parse_dues_to_cents(s: &str) -> Result<i64, String> {
    if s.is_empty() {
        return Ok(0);
    }
    if s.starts_with('-') {
        return Err("Dues amount cannot be negative.".into());
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if frac.len() > 2 {
        return Err("Dues amount supports at most 2 decimal places.".into());
    }
    let whole_n: i64 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| "Dues amount must be a number.".to_string())?
    };
    let frac_padded = format!("{:0<2}", frac);
    let frac_n: i64 = if frac_padded.is_empty() {
        0
    } else {
        frac_padded
            .parse()
            .map_err(|_| "Dues amount must be a number.".to_string())?
    };
    whole_n
        .checked_mul(100)
        .and_then(|w| w.checked_add(frac_n))
        .ok_or_else(|| "Dues amount is too large.".to_string())
}

fn is_iso_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    s.chars()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_underscore = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out.chars()
        .skip_while(|c| !c.is_ascii_lowercase())
        .take(41)
        .collect()
}

const TIMEZONES: &[&str] = &[
    "Asia/Kuala_Lumpur",
    "Asia/Singapore",
    "Asia/Jakarta",
    "Asia/Bangkok",
    "Asia/Tokyo",
    "Asia/Hong_Kong",
    "Asia/Kolkata",
    "Asia/Dubai",
    "Europe/London",
    "Europe/Berlin",
    "Europe/Paris",
    "America/New_York",
    "America/Los_Angeles",
    "Australia/Sydney",
    "UTC",
];

const CURRENCIES: &[(&str, &str)] = &[
    ("MYR", "Malaysian Ringgit"),
    ("SGD", "Singapore Dollar"),
    ("USD", "US Dollar"),
    ("EUR", "Euro"),
    ("GBP", "Pound Sterling"),
    ("JPY", "Japanese Yen"),
    ("INR", "Indian Rupee"),
    ("AUD", "Australian Dollar"),
    ("HKD", "Hong Kong Dollar"),
    ("IDR", "Indonesian Rupiah"),
    ("THB", "Thai Baht"),
    ("AED", "UAE Dirham"),
];

#[component]
pub fn CreateTenant() -> Element {
    let mut display_name = use_signal(String::new);
    let mut slug = use_signal(String::new);
    let mut slug_touched = use_signal(|| false);
    let mut timezone = use_signal(|| "Asia/Kuala_Lumpur".to_string());
    let mut currency = use_signal(|| "MYR".to_string());
    let mut cadence = use_signal(|| "monthly".to_string());
    // Held as the user-facing major-unit string so leading zeros / decimals
    // can be edited freely. Parsed at submit time.
    let mut dues_amount = use_signal(|| "0".to_string());
    // Optional carry-forward seed — for ventures that already had a kitty
    // before joining Sharam. Set ONCE at creation; the gateway enforces
    // write-once via DB EVENT.
    let mut cf_enabled = use_signal(|| false);
    let mut cf_from = use_signal(String::new);
    let mut cf_to = use_signal(String::new);
    let mut cf_amount = use_signal(String::new);
    let mut cf_note = use_signal(String::new);
    let mut form_error = use_signal(|| Option::<String>::None);
    let mut flash = use_signal(|| Option::<String>::None);
    let mut submitting = use_signal(|| false);

    let slug_issue = use_memo(move || {
        let s = slug.read();
        if s.is_empty() {
            None
        } else {
            validate_slug(&s).err()
        }
    });

    let on_name_input = move |e: FormEvent| {
        let v = e.value();
        if !*slug_touched.read() {
            slug.set(slugify(&v));
        }
        display_name.set(v);
        form_error.set(None);
    };

    let on_slug_input = move |e: FormEvent| {
        slug_touched.set(true);
        slug.set(e.value());
        form_error.set(None);
    };

    let submit = move |_| async move {
        let name = display_name.read().trim().to_string();
        let s = slug.read().trim().to_string();
        let tz = timezone.read().clone();
        let cur = currency.read().clone();
        let cad = cadence.read().clone();
        let dues_str = dues_amount.read().trim().to_string();

        if name.is_empty() {
            form_error.set(Some("Display name is required.".into()));
            return;
        }
        if let Err(SlugIssue(msg)) = validate_slug(&s) {
            form_error.set(Some(msg.into()));
            return;
        }
        let dues_cents = match parse_dues_to_cents(&dues_str) {
            Ok(c) => c,
            Err(msg) => {
                form_error.set(Some(msg));
                return;
            }
        };

        // Pre-validate carry-forward client-side so the user gets the error
        // before we burn the venture-create call. Carry-forward is set in a
        // second request — if we created the tenant and then bounced on bad
        // carry-forward input the user would have a tenant they didn't fully
        // configure (and the seed slot would still be available, just awkward).
        let cf_payload = if *cf_enabled.read() {
            let from = cf_from.read().trim().to_string();
            let to = cf_to.read().trim().to_string();
            let amount_str = cf_amount.read().trim().to_string();
            let note_raw = cf_note.read().trim().to_string();
            if !is_iso_date(&from) || !is_iso_date(&to) {
                form_error.set(Some(
                    "Carry-forward dates must be in YYYY-MM-DD format.".into(),
                ));
                return;
            }
            if from > to {
                form_error.set(Some(
                    "Carry-forward 'from' date must be on or before 'to' date.".into(),
                ));
                return;
            }
            let amount_cents = match parse_dues_to_cents(&amount_str) {
                Ok(c) if c > 0 => c,
                Ok(_) => {
                    form_error.set(Some(
                        "Carry-forward amount must be greater than zero.".into(),
                    ));
                    return;
                }
                Err(msg) => {
                    form_error.set(Some(format!("Carry-forward: {msg}")));
                    return;
                }
            };
            Some(CarryForwardRequest {
                from_date: from,
                to_date: to,
                amount_cents,
                note: if note_raw.is_empty() {
                    None
                } else {
                    Some(note_raw)
                },
            })
        } else {
            None
        };

        submitting.set(true);
        flash.set(None);
        form_error.set(None);

        let req = CreateTenantRequest {
            slug: s.clone(),
            display_name: name.clone(),
            timezone: tz,
            currency: cur,
            cadence: cad,
            dues_amount_cents: dues_cents,
        };

        match submit_tenant(req).await {
            Ok(resp) => {
                let mut msg = format!(
                    "Created '{name}' (slug: {}). You are the owner.",
                    resp.slug
                );
                // Seed the carry-forward in a follow-up call. If it fails we
                // still keep the tenant — the owner can retry the seed from
                // the manage page (TODO) or contact support.
                if let Some(cf) = cf_payload {
                    let cf_amount_cents = cf.amount_cents;
                    let cf_from_date = cf.from_date.clone();
                    let cf_to_date = cf.to_date.clone();
                    match submit_carry_forward(&resp.slug, cf).await {
                        Ok(()) => {
                            msg.push_str(&format!(
                                " Carry-forward seeded: {} cents covering {} → {}.",
                                cf_amount_cents, cf_from_date, cf_to_date
                            ));
                        }
                        Err(e) => {
                            form_error.set(Some(format!(
                                "Tenant created but carry-forward seed failed: {e}"
                            )));
                            submitting.set(false);
                            return;
                        }
                    }
                }
                flash.set(Some(msg));
                display_name.set(String::new());
                slug.set(String::new());
                slug_touched.set(false);
                dues_amount.set("0".to_string());
                cf_enabled.set(false);
                cf_from.set(String::new());
                cf_to.set(String::new());
                cf_amount.set(String::new());
                cf_note.set(String::new());
            }
            Err(e) => {
                form_error.set(Some(e.to_string()));
            }
        }
        submitting.set(false);
    };

    let preview_slug = slug.read().clone();
    let preview = if preview_slug.is_empty() {
        "your_slug".to_string()
    } else {
        preview_slug
    };

    let issue = slug_issue.read().clone();
    let busy = *submitting.read();

    rsx! {
        Sidenav { active: "create".to_string(),
            // Hero copy
            section {
                class: "px-6 lg:px-12 pt-10 pb-6 max-w-[1100px] rise",
                p { class: "eyebrow mb-4", "NEW LEDGER · YOU BECOME THE OWNER" }
                h1 {
                    class: "display text-[clamp(1.75rem,3.5vw,2.5rem)] font-light leading-[1.1] text-ink max-w-3xl",
                    "Open a new "
                    span {
                        class: "italic font-medium text-evergreen",
                        "venture"
                    }
                    "."
                }
                p {
                    class: "mt-3 text-[15px] text-ink-soft font-light max-w-2xl leading-[1.55]",
                    "The account you sign in with becomes the first owner — sole admin until you invite others. Each tenant lives in its own isolated namespace; the slug is permanent."
                }
            }

            // Two-column form
            div {
                class: "px-6 lg:px-12 pb-20 max-w-[1100px] grid lg:grid-cols-[1fr_360px] gap-6 rise",
                style: "animation-delay: 0.05s",

                // ── LEFT: form card ────────────────────────────────────
                section {
                    div {
                        class: "card p-7 lg:p-8",

                        h2 { class: "font-display text-[18px] font-semibold text-ink", "Tenant details" }
                        p {
                            class: "mt-1.5 text-[13.5px] text-ink-soft",
                            "All fields are required. The timezone governs how monthly periods close."
                        }

                        // Display name
                        div {
                            class: "mt-7",
                            label {
                                class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                r#for: "field-display-name",
                                "Display name"
                            }
                            input {
                                id: "field-display-name",
                                r#type: "text",
                                autocomplete: "off",
                                spellcheck: "false",
                                placeholder: "Helios Capital",
                                value: "{display_name}",
                                oninput: on_name_input,
                                class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink placeholder:text-ink-faint transition-all",
                            }
                            p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                "How this venture appears to its members."
                            }
                        }

                        // Slug
                        div {
                            class: "mt-5",
                            label {
                                class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                r#for: "field-slug",
                                "Slug "
                                span { class: "text-ink-faint font-normal", "(permanent identifier)" }
                            }
                            div {
                                class: "flex items-stretch border border-rule bg-paper rounded-md focus-within:border-evergreen focus-within:ring-4 focus-within:ring-evergreen/10 transition-all overflow-hidden",
                                span {
                                    class: "px-3 flex items-center bg-bone-soft border-r border-rule text-ink-faint font-mono text-[12.5px]",
                                    "sharam/"
                                }
                                input {
                                    id: "field-slug",
                                    r#type: "text",
                                    autocomplete: "off",
                                    spellcheck: "false",
                                    placeholder: "helios_capital",
                                    value: "{slug}",
                                    oninput: on_slug_input,
                                    class: "flex-1 bg-paper focus:outline-none px-3 py-2.5 font-mono text-[13.5px] text-ink",
                                }
                            }
                            if let Some(SlugIssue(msg)) = issue {
                                p { class: "text-[11.5px] text-negative mt-1.5", "{msg}" }
                            } else {
                                p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                    "3–41 chars · lowercase letters, digits, underscore · must start with a letter."
                                }
                            }
                        }

                        // Timezone + Currency
                        div {
                            class: "mt-5 grid grid-cols-1 sm:grid-cols-2 gap-4",

                            div {
                                label {
                                    class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                    r#for: "field-tz",
                                    "Timezone"
                                }
                                select {
                                    id: "field-tz",
                                    value: "{timezone}",
                                    onchange: move |e| timezone.set(e.value()),
                                    class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink transition-all",
                                    for tz in TIMEZONES.iter() {
                                        option { value: "{tz}", "{tz}" }
                                    }
                                }
                                p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                    "Determines when each monthly period closes."
                                }
                            }

                            div {
                                label {
                                    class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                    r#for: "field-currency",
                                    "Currency"
                                }
                                select {
                                    id: "field-currency",
                                    value: "{currency}",
                                    onchange: move |e| currency.set(e.value()),
                                    class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink transition-all",
                                    for (code, label) in CURRENCIES.iter() {
                                        option { value: "{code}", "{code} — {label}" }
                                    }
                                }
                                p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                    "Display currency for contributions."
                                }
                            }
                        }

                        // Cadence + Dues
                        div {
                            class: "mt-5 grid grid-cols-1 sm:grid-cols-2 gap-4",

                            div {
                                label {
                                    class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                    r#for: "field-cadence",
                                    "Dues cadence"
                                }
                                select {
                                    id: "field-cadence",
                                    value: "{cadence}",
                                    onchange: move |e| {
                                        cadence.set(e.value());
                                        form_error.set(None);
                                    },
                                    class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink transition-all",
                                    for (code, label) in CADENCES.iter() {
                                        option { value: "{code}", "{label}" }
                                    }
                                }
                                p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                    "How often each member owes dues. Editable later."
                                }
                            }

                            div {
                                label {
                                    class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                    r#for: "field-dues",
                                    "Dues per cycle"
                                }
                                div {
                                    class: "flex items-stretch border border-rule bg-paper rounded-md focus-within:border-evergreen focus-within:ring-4 focus-within:ring-evergreen/10 transition-all overflow-hidden",
                                    span {
                                        class: "px-3 flex items-center bg-bone-soft border-r border-rule text-ink-faint font-mono text-[12.5px]",
                                        "{currency}"
                                    }
                                    input {
                                        id: "field-dues",
                                        r#type: "text",
                                        inputmode: "decimal",
                                        autocomplete: "off",
                                        spellcheck: "false",
                                        placeholder: "0.00",
                                        value: "{dues_amount}",
                                        oninput: move |e| {
                                            dues_amount.set(e.value());
                                            form_error.set(None);
                                        },
                                        class: "flex-1 bg-paper focus:outline-none px-3 py-2.5 font-mono text-[13.5px] text-ink",
                                    }
                                }
                                p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                    "Amount each member owes per cycle. Editable later."
                                }
                            }
                        }

                        // Carry-forward (optional, write-once at creation)
                        div {
                            class: "mt-7 pt-6 border-t border-rule",
                            label {
                                class: "flex items-start gap-3 cursor-pointer",
                                input {
                                    r#type: "checkbox",
                                    checked: *cf_enabled.read(),
                                    onchange: move |e| {
                                        cf_enabled.set(e.value() == "true");
                                        form_error.set(None);
                                    },
                                    class: "mt-1 h-4 w-4 accent-evergreen",
                                }
                                div {
                                    p { class: "text-[13.5px] font-medium text-ink",
                                        "Seed with prior off-platform savings"
                                    }
                                    p { class: "text-[12px] text-ink-soft mt-0.5 leading-relaxed",
                                        "If your venture already collected money before joining Sharam, record it here as the starting balance. "
                                        span { class: "text-ink-faint italic",
                                            "Set once at creation — cannot be changed later."
                                        }
                                    }
                                }
                            }

                            if *cf_enabled.read() {
                                div {
                                    class: "mt-5 pl-7 space-y-4",

                                    // Date range
                                    div {
                                        class: "grid grid-cols-1 sm:grid-cols-2 gap-4",
                                        div {
                                            label {
                                                class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                                r#for: "field-cf-from",
                                                "Accumulated from"
                                            }
                                            input {
                                                id: "field-cf-from",
                                                r#type: "date",
                                                value: "{cf_from}",
                                                oninput: move |e| {
                                                    cf_from.set(e.value());
                                                    form_error.set(None);
                                                },
                                                class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink transition-all",
                                            }
                                        }
                                        div {
                                            label {
                                                class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                                r#for: "field-cf-to",
                                                "Accumulated to"
                                            }
                                            input {
                                                id: "field-cf-to",
                                                r#type: "date",
                                                value: "{cf_to}",
                                                oninput: move |e| {
                                                    cf_to.set(e.value());
                                                    form_error.set(None);
                                                },
                                                class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink transition-all",
                                            }
                                        }
                                    }

                                    // Amount
                                    div {
                                        label {
                                            class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                            r#for: "field-cf-amount",
                                            "Total amount carried forward"
                                        }
                                        div {
                                            class: "flex items-stretch border border-rule bg-paper rounded-md focus-within:border-evergreen focus-within:ring-4 focus-within:ring-evergreen/10 transition-all overflow-hidden",
                                            span {
                                                class: "px-3 flex items-center bg-bone-soft border-r border-rule text-ink-faint font-mono text-[12.5px]",
                                                "{currency}"
                                            }
                                            input {
                                                id: "field-cf-amount",
                                                r#type: "text",
                                                inputmode: "decimal",
                                                autocomplete: "off",
                                                spellcheck: "false",
                                                placeholder: "0.00",
                                                value: "{cf_amount}",
                                                oninput: move |e| {
                                                    cf_amount.set(e.value());
                                                    form_error.set(None);
                                                },
                                                class: "flex-1 bg-paper focus:outline-none px-3 py-2.5 font-mono text-[13.5px] text-ink",
                                            }
                                        }
                                        p { class: "text-[11.5px] text-ink-faint mt-1.5",
                                            "Aggregate sum across all members for the period above."
                                        }
                                    }

                                    // Optional note
                                    div {
                                        label {
                                            class: "block text-[12.5px] font-medium text-ink-soft mb-1.5",
                                            r#for: "field-cf-note",
                                            "Note "
                                            span { class: "text-ink-faint font-normal", "(optional)" }
                                        }
                                        input {
                                            id: "field-cf-note",
                                            r#type: "text",
                                            placeholder: "e.g. Carried over from informal monthly collection",
                                            value: "{cf_note}",
                                            oninput: move |e| cf_note.set(e.value()),
                                            class: "w-full bg-paper border border-rule focus:border-evergreen focus:outline-none focus:ring-4 focus:ring-evergreen/10 px-3.5 py-2.5 rounded-md text-[14px] text-ink placeholder:text-ink-faint transition-all",
                                        }
                                    }
                                }
                            }
                        }

                        // Submit
                        div {
                            class: "mt-8 flex items-center gap-4",
                            button {
                                r#type: "button",
                                disabled: busy,
                                onclick: submit,
                                class: "inline-flex items-center gap-2 bg-evergreen hover:bg-evergreen-deep disabled:opacity-50 disabled:cursor-not-allowed text-paper font-medium text-[14px] px-5 py-2.5 rounded-md transition-colors",
                                if busy { "Creating…" } else { "Create tenant" }
                                if !busy {
                                    span { class: "text-[16px] leading-none", "→" }
                                }
                            }
                            a {
                                href: "/",
                                class: "text-[13px] text-ink-soft hover:text-evergreen transition-colors",
                                "Cancel"
                            }
                        }

                        if let Some(msg) = form_error.read().clone() {
                            div {
                                class: "mt-5 px-3.5 py-2.5 rounded-md bg-negative-soft border border-negative/15",
                                p { class: "text-[12.5px] text-negative leading-relaxed", "{msg}" }
                            }
                        }

                        if let Some(msg) = flash.read().clone() {
                            div {
                                class: "mt-5 px-3.5 py-2.5 rounded-md bg-positive-soft border border-positive/15",
                                p { class: "text-[12.5px] text-positive leading-relaxed", "{msg}" }
                            }
                        }

                        div {
                            class: "mt-7 pt-5 border-t border-rule",
                            p { class: "text-[12px] text-ink-soft leading-relaxed",
                                "Calls "
                                code { class: "font-mono text-[11.5px] text-evergreen", "POST /api/tenants" }
                                " with your Google credential. The slug becomes a permanent SurrealDB namespace; you become the sole owner."
                            }
                        }
                    }
                }

                // ── RIGHT: summary panel ───────────────────────────────
                aside {
                    div {
                        class: "card p-6 sticky top-6",
                        p { class: "eyebrow mb-3", "PREVIEW" }

                        div {
                            class: "rounded-md bg-bone-soft border border-rule p-4",
                            p { class: "text-[11px] text-ink-faint font-mono uppercase tracking-[0.14em]", "Namespace" }
                            p {
                                class: "mt-1 font-mono text-[14px] text-ink break-all",
                                "ns="
                                span { class: "text-evergreen", "{preview}" }
                                " db=main"
                            }
                        }

                        div {
                            class: "mt-5 space-y-3 text-[13px]",
                            SummaryRow {
                                label: "Display".to_string(),
                                value: if display_name.read().is_empty() {
                                    "—".to_string()
                                } else {
                                    display_name.read().clone()
                                },
                            }
                            SummaryRow { label: "Timezone".to_string(), value: timezone.read().clone() }
                            SummaryRow { label: "Currency".to_string(), value: currency.read().clone() }
                            SummaryRow {
                                label: "Cadence".to_string(),
                                value: CADENCES
                                    .iter()
                                    .find(|(code, _)| *code == cadence.read().as_str())
                                    .map(|(_, label)| (*label).to_string())
                                    .unwrap_or_else(|| cadence.read().clone()),
                            }
                            SummaryRow {
                                label: "Dues / cycle".to_string(),
                                value: format!("{} {}", currency.read(), dues_amount.read()),
                            }
                            SummaryRow { label: "Your role".to_string(), value: "Owner".to_string() }
                        }

                        div {
                            class: "mt-6 pt-5 border-t border-rule",
                            p { class: "eyebrow mb-2", "AFTER CREATE" }
                            ul {
                                class: "list-disc pl-4 text-[12.5px] text-ink-soft leading-[1.6] space-y-1",
                                li { "You become the sole owner-member." }
                                li { "Period locking starts from this month." }
                                li { "Invite treasurers and members from Admin." }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SummaryRow(label: String, value: String) -> Element {
    rsx! {
        div {
            class: "flex items-baseline justify-between gap-3",
            span { class: "text-ink-faint text-[11.5px] uppercase tracking-[0.12em] font-mono", "{label}" }
            span { class: "text-ink text-[13.5px] font-medium text-right break-all", "{value}" }
        }
    }
}
