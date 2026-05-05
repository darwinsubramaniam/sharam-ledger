use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] surrealdb::Error),

    #[error("period {period} is locked and cannot be modified")]
    PeriodLocked { period: String },

    #[error("dues cap exceeded: paid {paid_cents} > dues {dues_cents} (cents)")]
    DuesCapExceeded { paid_cents: i64, dues_cents: i64 },

    #[error("tenant {0} already exists")]
    TenantExists(String),

    #[error("invite already exists for {email} in {slug}")]
    InviteExists { slug: String, email: String },

    #[error("user with email {0} already has a password set")]
    UserExists(String),

    #[error("carry-forward seed already set for this venture")]
    CarryForwardExists,

    #[error("not found")]
    NotFound,

    #[error("invariant: {0}")]
    Invariant(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Inspect a `surrealdb::Error` for an event throw and reshape it. Anything
/// not recognised falls through as `Error::Db`.
pub fn map_db_error(e: surrealdb::Error) -> Error {
    let msg = e.to_string();
    if let Some((_, rest)) = msg.split_once("period_locked:") {
        // "period_locked: 2026-03 < 2026-04"  →  pull "2026-03"
        let period = rest.split('<').next().unwrap_or("").trim().to_string();
        return Error::PeriodLocked { period };
    }
    if let Some((_, rest)) = msg.split_once("dues_cap_exceeded:") {
        // "dues_cap_exceeded: 12000 > 10000"  →  (12000, 10000)
        let mut parts = rest.split('>').map(str::trim);
        let paid_cents = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let dues_cents = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        return Error::DuesCapExceeded {
            paid_cents,
            dues_cents,
        };
    }
    if msg.contains("carry_forward_immutable") {
        return Error::CarryForwardExists;
    }
    // Re-CREATE on `carry_forward:current` lands here as a duplicate-record
    // error from SurrealDB. Reshape it so the gateway can return 409 instead
    // of a generic 500.
    if msg.contains("Database record `carry_forward:current` already exists") {
        return Error::CarryForwardExists;
    }
    Error::Db(e)
}
