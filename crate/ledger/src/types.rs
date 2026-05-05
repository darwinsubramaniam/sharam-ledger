use chrono::{DateTime, Utc};
use surrealdb::types::{RecordId, SurrealValue};

use common::domain::TenantSlug;

/// Stored row for `user` (control plane).
#[derive(Debug, Clone, SurrealValue)]
pub struct UserRecord {
    pub id: RecordId,
    pub email: String,
    pub google_sub: Option<String>,
    pub display_name: Option<String>,
    pub display_tz: String,
    pub created_at: DateTime<Utc>,
}

/// Stored row for `membership` (control plane).
/// `role` stays as a String here so this crate doesn't have to teach
/// `SurrealValue` to common's `Role` enum. The gateway can map to
/// `common::domain::Role` at the wire boundary.
#[derive(Debug, Clone, SurrealValue)]
pub struct MembershipRecord {
    pub id: RecordId,
    pub user: RecordId,
    pub tenant_slug: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Tenant directory entry (control plane).
#[derive(Debug, Clone, SurrealValue)]
pub struct TenantDirectoryRecord {
    pub id: RecordId,
    pub slug: String,
    pub display_name: String,
    pub created_by: RecordId,
    pub created_at: DateTime<Utc>,
}

/// Read-only projection: a member of a tenant joined with their user row.
/// Built by `Ledger::list_tenant_members` from `membership` + `user`.
#[derive(Debug, Clone)]
pub struct TenantMember {
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

/// Read-only projection for the dashboard: a tenant a user belongs to,
/// joined with the directory's display_name and the caller's role.
#[derive(Debug, Clone)]
pub struct VentureSummary {
    pub slug: String,
    pub display_name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// Stored row for `invite` (control plane). Gates which Google emails may
/// join which tenant. `role` stays a String for the same reason as
/// `MembershipRecord` — gateway maps to `common::domain::Role` at the wire.
#[derive(Debug, Clone, SurrealValue)]
pub struct InviteRecord {
    pub id: RecordId,
    pub tenant_slug: String,
    pub email: String,
    pub role: String,
    pub invited_by: RecordId,
    pub created_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Input to `Ledger::create_invite`.
#[derive(Debug, Clone)]
pub struct NewInvite {
    pub tenant_slug: String,
    pub email: String,
    pub role: String,
    pub invited_by: RecordId,
}

/// Stored row for `settings:current` (per-tenant). `cadence` is the venture's
/// dues frequency; `dues_amount_cents` is the per-cycle amount each member
/// owes. Both are mutable post-creation.
#[derive(Debug, Clone, SurrealValue)]
pub struct TenantSettings {
    pub timezone: String,
    pub currency: String,
    pub display_name: String,
    pub cadence: String,
    pub dues_amount_cents: i64,
}

/// Stored row for `contribution` (per-tenant). `cadence` is snapshotted at
/// write-time so historical rows survive cadence changes on settings.
#[derive(Debug, Clone, SurrealValue)]
pub struct ContributionRecord {
    pub id: RecordId,
    pub user_email: String,
    pub cadence: String,
    pub period: String,
    pub amount_cents: i64,
    pub status: String,
    pub proof_key: Option<String>,
    pub note: Option<String>,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub locked_at: Option<DateTime<Utc>>,
}

/// Input to `Ledger::create_tenant`.
#[derive(Debug, Clone)]
pub struct NewTenant {
    pub slug: TenantSlug,
    pub display_name: String,
    pub timezone: String,
    pub currency: String,
    pub cadence: String,
    pub dues_amount_cents: i64,
    /// User who is creating this tenant. Becomes the first owner-membership.
    pub created_by: RecordId,
}

/// Input to `Ledger::add_contribution`. `cadence` MUST match the row's
/// period shape — the gateway should read `settings:current.cadence` and
/// pass it through; the DB-side lock event uses it to compute the current
/// period for comparison. The DB-side dues-cap event will reject the
/// write if the resulting non-rejected sum exceeds dues_amount_cents.
#[derive(Debug, Clone)]
pub struct NewContribution {
    pub user_email: String,
    pub cadence: String,
    pub period: String,
    pub amount_cents: i64,
    pub proof_key: Option<String>,
    pub note: Option<String>,
}

/// Read-only projection: rolled-up payment state for a single
/// (user_email, period). `paid_cents` is the sum across non-rejected rows.
#[derive(Debug, Clone, Copy)]
pub struct PeriodSummary {
    pub dues_cents: i64,
    pub paid_cents: i64,
    pub remaining_cents: i64,
}

/// One non-rejected payment expressed as `(when, how_much)`. Returned by
/// `Ledger::accumulation_points` in submitted-at order so the gateway can
/// bucket and running-sum without additional sorting.
#[derive(Debug, Clone, SurrealValue)]
pub struct AccumulationPoint {
    pub submitted_at: DateTime<Utc>,
    pub amount_cents: i64,
}

/// Input to `Ledger::upsert_user`.
#[derive(Debug, Clone)]
pub struct UpsertUser {
    pub email: String,
    pub google_sub: String,
    pub display_name: Option<String>,
}

/// Patch input for `Ledger::update_settings`. Each `Some(_)` field is
/// applied; `None` leaves the existing value untouched (MERGE semantics).
#[derive(Debug, Clone, Default)]
pub struct UpdateSettings {
    pub display_name: Option<String>,
    pub timezone: Option<String>,
    pub currency: Option<String>,
    pub cadence: Option<String>,
    pub dues_amount_cents: Option<i64>,
}
