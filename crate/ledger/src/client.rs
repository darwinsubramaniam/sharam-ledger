use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use tracing::{debug, info};

use common::config::SurrealConfig;
use common::domain::TenantSlug;

use crate::error::{Error, Result, map_db_error};

fn endpoint_needs_auth(endpoint: &str) -> bool {
    endpoint.starts_with("ws://")
        || endpoint.starts_with("wss://")
        || endpoint.starts_with("http://")
        || endpoint.starts_with("https://")
}
use crate::sql::{CONTROL_SCHEMA, TENANT_SCHEMA};
use crate::types::{
    AccumulationPoint, ContributionRecord, InviteRecord, MembershipRecord, NewContribution,
    NewInvite, NewTenant, PeriodSummary, TenantDirectoryRecord, TenantMember, TenantSettings,
    UpdateSettings, UpsertUser, UserRecord, VentureSummary,
};

/// Ledger client. Owns one persistent connection to the control plane and
/// lazily opens one per-tenant connection (each bound to its own
/// `ns=<slug>, db=main`). Cheap to clone — the inner state is `Arc`-ed.
#[derive(Clone)]
pub struct Ledger {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: SurrealConfig,
    control: Surreal<Any>,
    tenants: Mutex<HashMap<String, Surreal<Any>>>,
}

impl Ledger {
    pub async fn connect(cfg: SurrealConfig) -> Result<Self> {
        let control = Self::connect_to(&cfg, &cfg.namespace, &cfg.database).await?;
        info!(ns = %cfg.namespace, db = %cfg.database, "ledger control plane connected");
        Ok(Self {
            inner: Arc::new(Inner {
                cfg,
                control,
                tenants: Mutex::default(),
            }),
        })
    }

    /// Apply the control-plane schema (idempotent — every DEFINE uses OVERWRITE).
    pub async fn apply_control_schema(&self) -> Result<()> {
        let mut resp = self.inner.control.query(CONTROL_SCHEMA).await?;
        let errs = resp.take_errors();
        if !errs.is_empty() {
            return Err(Error::Invariant(format!(
                "control schema apply failed: {errs:?}"
            )));
        }
        debug!("control schema applied");
        Ok(())
    }

    /// Provision a new tenant: creates `ns=<slug> db=main`, applies the
    /// tenant schema, writes `settings:current`, and registers the tenant
    /// in the control-plane directory + first owner membership.
    pub async fn create_tenant(&self, init: NewTenant) -> Result<()> {
        // 1. Reject if the slug already exists in the directory.
        let existing: Option<TenantDirectoryRecord> = self
            .inner
            .control
            .query("SELECT * FROM ONLY tenant_directory WHERE slug = $slug LIMIT 1")
            .bind(("slug", init.slug.as_str().to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        if existing.is_some() {
            return Err(Error::TenantExists(init.slug.as_str().into()));
        }

        // 2. Connect to the new namespace; SurrealDB creates ns/db on first
        //    DEFINE inside it.
        let tenant_db = Self::connect_to(&self.inner.cfg, init.slug.as_str(), "main").await?;

        // 3. Apply tenant schema.
        let mut resp = tenant_db.query(TENANT_SCHEMA).await.map_err(map_db_error)?;
        let errs = resp.take_errors();
        if !errs.is_empty() {
            return Err(Error::Invariant(format!(
                "tenant schema apply failed: {errs:?}"
            )));
        }

        // 4. Write settings:current.
        tenant_db
            .query(
                "CREATE settings:current SET \
                    display_name = $display_name, \
                    timezone = $timezone, \
                    currency = $currency, \
                    cadence = $cadence, \
                    dues_amount_cents = $dues_amount_cents",
            )
            .bind(("display_name", init.display_name.clone()))
            .bind(("timezone", init.timezone.clone()))
            .bind(("currency", init.currency.clone()))
            .bind(("cadence", init.cadence.clone()))
            .bind(("dues_amount_cents", init.dues_amount_cents))
            .await
            .map_err(map_db_error)?
            .check()?;

        // 5. Cache the tenant connection.
        self.inner
            .tenants
            .lock()
            .expect("tenant cache poisoned")
            .insert(init.slug.as_str().to_string(), tenant_db);

        // 6. Register in the control directory + owner membership.
        self.inner
            .control
            .query(
                "BEGIN; \
                 CREATE tenant_directory SET \
                    slug = $slug, display_name = $display_name, created_by = $created_by; \
                 CREATE membership SET \
                    user = $created_by, tenant_slug = $slug, role = 'owner'; \
                 COMMIT;",
            )
            .bind(("slug", init.slug.as_str().to_string()))
            .bind(("display_name", init.display_name))
            .bind(("created_by", init.created_by))
            .await
            .map_err(map_db_error)?
            .check()?;

        info!(slug = %init.slug, "tenant created");
        Ok(())
    }

    /// Upsert the Google identity into the control plane. Returns the
    /// canonical row (whether newly created or existing).
    pub async fn upsert_user(&self, input: UpsertUser) -> Result<UserRecord> {
        let row: Option<UserRecord> = self
            .inner
            .control
            .query(
                "UPSERT ONLY user \
                    SET email = $email, \
                        google_sub = $google_sub, \
                        display_name = $display_name \
                    WHERE email = $email \
                    RETURN AFTER",
            )
            .bind(("email", input.email))
            .bind(("google_sub", input.google_sub))
            .bind(("display_name", input.display_name))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        row.ok_or(Error::NotFound)
    }

    /// All tenants (with role) accessible to the user owning this email.
    pub async fn list_memberships_for(&self, email: &str) -> Result<Vec<MembershipRecord>> {
        let rows: Vec<MembershipRecord> = self
            .inner
            .control
            .query(
                "SELECT * FROM membership \
                 WHERE user IN (SELECT VALUE id FROM user WHERE email = $email)",
            )
            .bind(("email", email.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(rows)
    }

    /// Look up the human display name for a tenant via the control-plane
    /// directory. Returns `None` when the slug isn't registered (the
    /// directory is a cache, not the source of truth — callers may fall back
    /// to the slug).
    pub async fn tenant_display_name(&self, slug: &str) -> Result<Option<String>> {
        let row: Option<TenantDirectoryRecord> = self
            .inner
            .control
            .query("SELECT * FROM ONLY tenant_directory WHERE slug = $slug LIMIT 1")
            .bind(("slug", slug.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(row.map(|r| r.display_name))
    }

    /// Dashboard view: every venture this user belongs to, joined with the
    /// directory's display_name. Newest membership first.
    pub async fn list_user_ventures(&self, email: &str) -> Result<Vec<VentureSummary>> {
        let memberships = self.list_memberships_for(email).await?;
        if memberships.is_empty() {
            return Ok(vec![]);
        }
        let slugs: Vec<String> = memberships.iter().map(|m| m.tenant_slug.clone()).collect();
        let dirs: Vec<TenantDirectoryRecord> = self
            .inner
            .control
            .query("SELECT * FROM tenant_directory WHERE slug IN $slugs")
            .bind(("slugs", slugs))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        let by_slug: HashMap<String, String> =
            dirs.into_iter().map(|d| (d.slug, d.display_name)).collect();
        let mut out: Vec<VentureSummary> = memberships
            .into_iter()
            .map(|m| VentureSummary {
                display_name: by_slug
                    .get(&m.tenant_slug)
                    .cloned()
                    .unwrap_or_else(|| m.tenant_slug.clone()),
                slug: m.tenant_slug,
                role: m.role,
                created_at: m.created_at,
            })
            .collect();
        out.sort_by_key(|v| std::cmp::Reverse(v.created_at));
        Ok(out)
    }

    /// Materialize any pending (non-revoked, non-accepted) invites for
    /// `email` into memberships and mark them accepted. Idempotent — re-runs
    /// on every sign-in.
    ///
    /// For each pending invite this performs, **atomically**:
    ///   1. UPSERT a `membership` row keyed by `(user, tenant_slug)` so a
    ///      pre-existing row is left alone (or its `role` updated) instead
    ///      of triggering a unique-index violation.
    ///   2. Mark the invite `accepted_at = time::now()`.
    ///
    /// If the membership write fails the invite is **not** marked accepted
    /// — that way a transient failure can be retried on the next sign-in
    /// instead of leaving the invite "accepted" but without a membership
    /// row (the bug previously seen).
    ///
    /// Returns the list of tenant slugs newly accepted.
    pub async fn accept_pending_invites(
        &self,
        email: &str,
        user_id: surrealdb::types::RecordId,
    ) -> Result<Vec<String>> {
        // Also pulls already-accepted invites so we can reconcile orphan
        // rows where a prior acceptance attempt set `accepted_at` but the
        // membership write didn't land. UPSERT below makes that idempotent.
        let pending: Vec<InviteRecord> = self
            .inner
            .control
            .query(
                "SELECT * FROM invite \
                 WHERE email = $email AND revoked_at IS NONE",
            )
            .bind(("email", email.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;

        if pending.is_empty() {
            return Ok(vec![]);
        }
        info!(
            email = %email,
            pending = pending.len(),
            "processing invites"
        );

        let mut accepted = Vec::with_capacity(pending.len());
        for inv in pending {
            let was_accepted = inv.accepted_at.is_some();
            // 1. Upsert membership. UPSERT handles the "already a member"
            //    case naturally — no unique-index violation, no SELECT-then-
            //    CREATE race window.
            let mut membership_resp = self
                .inner
                .control
                .query(
                    "UPSERT ONLY membership \
                        SET user = $user, tenant_slug = $slug, role = $role \
                        WHERE user = $user AND tenant_slug = $slug \
                        RETURN AFTER",
                )
                .bind(("user", user_id.clone()))
                .bind(("slug", inv.tenant_slug.clone()))
                .bind(("role", inv.role.clone()))
                .await
                .map_err(map_db_error)?;
            let mem_errs = membership_resp.take_errors();
            if let Some((_, e)) = mem_errs.into_iter().next() {
                return Err(map_db_error(e));
            }
            let row: Option<MembershipRecord> = membership_resp.take(0)?;
            if row.is_none() {
                return Err(Error::Invariant(format!(
                    "membership upsert returned no row for slug={} email={}",
                    inv.tenant_slug, email
                )));
            }
            info!(
                email = %email,
                slug = %inv.tenant_slug,
                role = %inv.role,
                "membership materialized from invite"
            );

            // 2. Mark invite accepted only after the membership is in place.
            //    Skip if already accepted so we don't clobber the original
            //    timestamp on a reconcile pass.
            if !was_accepted {
                self.inner
                    .control
                    .query("UPDATE ONLY $id SET accepted_at = time::now()")
                    .bind(("id", inv.id))
                    .await
                    .map_err(map_db_error)?
                    .check()?;
                accepted.push(inv.tenant_slug);
            }
        }
        Ok(accepted)
    }

    /// Every member of `slug` joined with their user row, oldest membership
    /// first (so the founding owner shows at the top). The join is done
    /// client-side because SurrealDB FETCH would change the result shape and
    /// `MembershipRecord.user` is a `RecordId`.
    pub async fn list_tenant_members(&self, slug: &str) -> Result<Vec<TenantMember>> {
        let memberships: Vec<MembershipRecord> = self
            .inner
            .control
            .query("SELECT * FROM membership WHERE tenant_slug = $slug ORDER BY created_at ASC")
            .bind(("slug", slug.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        if memberships.is_empty() {
            return Ok(vec![]);
        }
        let user_ids: Vec<surrealdb::types::RecordId> =
            memberships.iter().map(|m| m.user.clone()).collect();
        let users: Vec<UserRecord> = self
            .inner
            .control
            .query("SELECT * FROM user WHERE id IN $ids")
            .bind(("ids", user_ids))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        let by_id: HashMap<surrealdb::types::RecordId, UserRecord> =
            users.into_iter().map(|u| (u.id.clone(), u)).collect();
        Ok(memberships
            .into_iter()
            .map(|m| {
                let user = by_id.get(&m.user);
                TenantMember {
                    email: user.map(|u| u.email.clone()).unwrap_or_default(),
                    display_name: user.and_then(|u| u.display_name.clone()),
                    role: m.role,
                    joined_at: m.created_at,
                }
            })
            .collect())
    }

    /// Create or reactivate an invite for `(tenant_slug, email)`.
    ///
    /// The schema enforces uniqueness on `(tenant_slug, email)`, so a
    /// previously-revoked row would block a fresh CREATE. We instead:
    ///   - If an active (non-revoked) row exists → `InviteExists` (409).
    ///   - If a revoked row exists → reactivate it (clear `revoked_at`,
    ///     update `role` + `invited_by`). `created_at` is `READONLY` so it
    ///     stays as the original invite date.
    ///   - Otherwise → CREATE.
    ///
    /// Caller is responsible for verifying that `invited_by` is an owner of
    /// the tenant — the ledger does not enforce roles.
    pub async fn create_invite(&self, input: NewInvite) -> Result<InviteRecord> {
        let existing: Option<InviteRecord> = self
            .inner
            .control
            .query(
                "SELECT * FROM ONLY invite \
                 WHERE tenant_slug = $slug AND email = $email \
                 LIMIT 1",
            )
            .bind(("slug", input.tenant_slug.clone()))
            .bind(("email", input.email.clone()))
            .await
            .map_err(map_db_error)?
            .take(0)?;

        if let Some(rec) = existing {
            if rec.revoked_at.is_none() {
                return Err(Error::InviteExists {
                    slug: input.tenant_slug,
                    email: input.email,
                });
            }
            // Reactivate the revoked row.
            let row: Option<InviteRecord> = self
                .inner
                .control
                .query(
                    "UPDATE ONLY $id SET \
                        revoked_at = NONE, \
                        role = $role, \
                        invited_by = $invited_by \
                     RETURN AFTER",
                )
                .bind(("id", rec.id))
                .bind(("role", input.role))
                .bind(("invited_by", input.invited_by))
                .await
                .map_err(map_db_error)?
                .take(0)?;
            return row.ok_or(Error::NotFound);
        }

        let row: Option<InviteRecord> = self
            .inner
            .control
            .query(
                "CREATE ONLY invite SET \
                    tenant_slug = $slug, \
                    email = $email, \
                    role = $role, \
                    invited_by = $invited_by \
                 RETURN AFTER",
            )
            .bind(("slug", input.tenant_slug))
            .bind(("email", input.email))
            .bind(("role", input.role))
            .bind(("invited_by", input.invited_by))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        row.ok_or(Error::NotFound)
    }

    /// Every invite ever issued for `slug`, newest first. Includes accepted
    /// and revoked rows so the admin UI can show the full history.
    pub async fn list_invites_for_tenant(&self, slug: &str) -> Result<Vec<InviteRecord>> {
        let rows: Vec<InviteRecord> = self
            .inner
            .control
            .query("SELECT * FROM invite WHERE tenant_slug = $slug ORDER BY created_at DESC")
            .bind(("slug", slug.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(rows)
    }

    /// Hard-delete an invite. Only succeeds if the invite is **not**
    /// accepted — once accepted there's a `membership` row depending on it
    /// and the invite stays as audit history. Returns `NotFound` if the
    /// row is missing, accepted, or belongs to a different tenant.
    pub async fn delete_invite(&self, slug: &str, invite_key: &str) -> Result<()> {
        let deleted: Vec<InviteRecord> = self
            .inner
            .control
            .query(
                "DELETE type::record(\"invite\", $key) \
                 WHERE tenant_slug = $slug AND accepted_at IS NONE \
                 RETURN BEFORE",
            )
            .bind(("key", invite_key.to_string()))
            .bind(("slug", slug.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        if deleted.is_empty() {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    /// Mark an invite revoked. `invite_key` is the record-id key portion
    /// (the part after `invite:`). The `(slug, key)` pair is matched so a
    /// caller cannot revoke an invite that doesn't belong to the tenant
    /// they have permission over.
    pub async fn revoke_invite(&self, slug: &str, invite_key: &str) -> Result<InviteRecord> {
        let row: Option<InviteRecord> = self
            .inner
            .control
            .query(
                "UPDATE ONLY type::record(\"invite\", $key) \
                 SET revoked_at = time::now() \
                 WHERE tenant_slug = $slug AND revoked_at IS NONE \
                 RETURN AFTER",
            )
            .bind(("key", invite_key.to_string()))
            .bind(("slug", slug.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        row.ok_or(Error::NotFound)
    }

    /// Append a single payment toward `(user_email, period)`. Each call
    /// CREATEs a new row with a fresh UUIDv7 id — partial payments are
    /// modelled as multiple rows that sum to the dues amount. The DB-side
    /// lock event rejects writes whose `period` is older than the tenant's
    /// current period; the dues-cap event rejects writes that would push
    /// the non-rejected sum past `settings:current.dues_amount_cents`.
    pub async fn add_contribution(
        &self,
        slug: &TenantSlug,
        input: NewContribution,
    ) -> Result<ContributionRecord> {
        let tenant = self.tenant_db(slug).await?;
        let mut resp = tenant
            .query(
                "CREATE ONLY contribution SET \
                    user_email = $email, \
                    cadence = $cadence, \
                    period = $period, \
                    amount_cents = $amount, \
                    proof_key = $proof_key, \
                    note = $note \
                 RETURN AFTER",
            )
            .bind(("email", input.user_email))
            .bind(("cadence", input.cadence))
            .bind(("period", input.period))
            .bind(("amount", input.amount_cents))
            .bind(("proof_key", input.proof_key))
            .bind(("note", input.note))
            .await
            .map_err(map_db_error)?;
        let errs = resp.take_errors();
        if let Some((_, e)) = errs.into_iter().next() {
            return Err(map_db_error(e));
        }
        let row: Option<ContributionRecord> = resp.take(0)?;
        row.ok_or(Error::NotFound)
    }

    /// All payments by `email` for `period` in `slug`, oldest first.
    /// Includes rejected rows so the UI can show the audit trail.
    pub async fn list_contributions(
        &self,
        slug: &TenantSlug,
        email: &str,
        period: &str,
    ) -> Result<Vec<ContributionRecord>> {
        let tenant = self.tenant_db(slug).await?;
        let rows: Vec<ContributionRecord> = tenant
            .query(
                "SELECT * FROM contribution \
                 WHERE user_email = $email AND period = $period \
                 ORDER BY submitted_at ASC",
            )
            .bind(("email", email.to_string()))
            .bind(("period", period.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(rows)
    }

    /// All payments made by every member for `period` in `slug`, oldest
    /// first. Includes rejected rows so the audit trail is honest. Used by
    /// the venture-wide pool view — every member is allowed to read this.
    pub async fn list_pool_contributions(
        &self,
        slug: &TenantSlug,
        period: &str,
    ) -> Result<Vec<ContributionRecord>> {
        let tenant = self.tenant_db(slug).await?;
        let rows: Vec<ContributionRecord> = tenant
            .query(
                "SELECT * FROM contribution \
                 WHERE period = $period \
                 ORDER BY submitted_at ASC",
            )
            .bind(("period", period.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(rows)
    }

    /// Distinct `period` values that have at least one contribution row
    /// (rejected or not), newest first. Powers the period dropdown.
    pub async fn list_active_periods(&self, slug: &TenantSlug) -> Result<Vec<String>> {
        let tenant = self.tenant_db(slug).await?;
        let raw: Vec<String> = tenant
            .query("SELECT VALUE period FROM contribution")
            .await
            .map_err(map_db_error)?
            .take(0)?;
        let uniq: std::collections::BTreeSet<String> = raw.into_iter().collect();
        // BTreeSet iter is ascending; reverse for newest-first.
        Ok(uniq.into_iter().rev().collect())
    }

    /// Raw `(submitted_at, amount_cents)` points across all non-rejected
    /// contributions in `slug`, oldest first. The gateway buckets these
    /// into the accumulation series the chart renders.
    pub async fn accumulation_points(
        &self,
        slug: &TenantSlug,
    ) -> Result<Vec<AccumulationPoint>> {
        let tenant = self.tenant_db(slug).await?;
        let rows: Vec<AccumulationPoint> = tenant
            .query(
                "SELECT submitted_at, amount_cents FROM contribution \
                 WHERE status != 'rejected' \
                 ORDER BY submitted_at ASC",
            )
            .await
            .map_err(map_db_error)?
            .take(0)?;
        Ok(rows)
    }

    /// Roll-up of `(email, period)` payments against the tenant's current
    /// dues amount. Excludes rejected rows. `remaining_cents` is clamped
    /// at zero (it can never go negative — the dues-cap event prevents
    /// overpayment).
    pub async fn period_summary(
        &self,
        slug: &TenantSlug,
        email: &str,
        period: &str,
    ) -> Result<PeriodSummary> {
        let tenant = self.tenant_db(slug).await?;
        let dues_cents = self.tenant_settings(slug).await?.dues_amount_cents;
        let amounts: Vec<i64> = tenant
            .query(
                "SELECT VALUE amount_cents FROM contribution \
                 WHERE user_email = $email \
                   AND period = $period \
                   AND status != 'rejected'",
            )
            .bind(("email", email.to_string()))
            .bind(("period", period.to_string()))
            .await
            .map_err(map_db_error)?
            .take(0)?;
        let paid_cents: i64 = amounts.iter().sum();
        let remaining_cents = (dues_cents - paid_cents).max(0);
        Ok(PeriodSummary {
            dues_cents,
            paid_cents,
            remaining_cents,
        })
    }

    /// Read the tenant's `settings:current` singleton.
    pub async fn tenant_settings(&self, slug: &TenantSlug) -> Result<TenantSettings> {
        let tenant = self.tenant_db(slug).await?;
        let s: Option<TenantSettings> = tenant
            .query("SELECT * FROM ONLY settings:current")
            .await
            .map_err(map_db_error)?
            .take(0)?;
        s.ok_or(Error::NotFound)
    }

    /// Patch `settings:current`. Only `Some(_)` fields are written; others
    /// are left untouched (SurrealDB MERGE semantics). Returns the row
    /// after the update. Authorization (owner-only, etc.) is the caller's
    /// responsibility — the ledger does not know about Roles.
    pub async fn update_settings(
        &self,
        slug: &TenantSlug,
        patch: UpdateSettings,
    ) -> Result<TenantSettings> {
        let tenant = self.tenant_db(slug).await?;

        let mut obj = serde_json::Map::new();
        if let Some(v) = patch.display_name {
            obj.insert("display_name".into(), v.into());
        }
        if let Some(v) = patch.timezone {
            obj.insert("timezone".into(), v.into());
        }
        if let Some(v) = patch.currency {
            obj.insert("currency".into(), v.into());
        }
        if let Some(v) = patch.cadence {
            obj.insert("cadence".into(), v.into());
        }
        if let Some(v) = patch.dues_amount_cents {
            obj.insert("dues_amount_cents".into(), v.into());
        }
        if obj.is_empty() {
            // Nothing to do — return current state instead of issuing an UPDATE
            // that would be a no-op but still bump `updated_at`.
            return self.tenant_settings(slug).await;
        }

        let mut resp = tenant
            .query("UPDATE ONLY settings:current MERGE $patch RETURN AFTER")
            .bind(("patch", serde_json::Value::Object(obj)))
            .await
            .map_err(map_db_error)?;
        let errs = resp.take_errors();
        if let Some((_, e)) = errs.into_iter().next() {
            return Err(map_db_error(e));
        }
        let row: Option<TenantSettings> = resp.take(0)?;
        row.ok_or(Error::NotFound)
    }

    // ─── internals ──────────────────────────────────────────────────────────

    async fn connect_to(cfg: &SurrealConfig, ns: &str, db: &str) -> Result<Surreal<Any>> {
        let conn = surrealdb::engine::any::connect(&cfg.endpoint).await?;
        // Embedded engines (memory/file/rocksdb/surrealkv) start with no auth
        // configured, so signin would fail. Only sign in for remote schemes.
        if endpoint_needs_auth(&cfg.endpoint) {
            conn.signin(Root {
                username: cfg.username.clone(),
                password: cfg.password.clone(),
            })
            .await?;
        }
        conn.use_ns(ns).use_db(db).await?;
        Ok(conn)
    }

    async fn tenant_db(&self, slug: &TenantSlug) -> Result<Surreal<Any>> {
        if let Some(c) = self
            .inner
            .tenants
            .lock()
            .expect("tenant cache poisoned")
            .get(slug.as_str())
        {
            return Ok(c.clone());
        }
        let conn = Self::connect_to(&self.inner.cfg, slug.as_str(), "main").await?;
        self.inner
            .tenants
            .lock()
            .expect("tenant cache poisoned")
            .insert(slug.as_str().to_string(), conn.clone());
        Ok(conn)
    }
}
