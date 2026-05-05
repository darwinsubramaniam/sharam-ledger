//! End-to-end smoke test against an in-memory SurrealDB.
//!
//! Exercises:
//!   1. Control-plane schema applies cleanly.
//!   2. A new tenant can be created (namespace + schema + settings + directory).
//!   3. The DB-side lock event rejects writes for past periods.
//!
//! Run with `cargo test -p ledger`.

use common::config::SurrealConfig;
use common::domain::TenantSlug;
use ledger::{Ledger, NewContribution, NewInvite, NewTenant, UpdateSettings, UpsertUser};

fn cfg() -> SurrealConfig {
    SurrealConfig {
        endpoint: "memory".to_string(),
        namespace: "system".to_string(),
        database: "control".to_string(),
        username: "root".to_string(),
        password: "root".to_string(),
    }
}

#[tokio::test]
async fn control_schema_applies() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.expect("schema");

    // Idempotency check.
    l.apply_control_schema().await.expect("re-apply");
}

#[tokio::test]
async fn migrate_all_tenants_preserves_existing_rows() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "preserve@example.com".into(),
            google_sub: "g_preserve_1".into(),
            display_name: None,
        })
        .await
        .unwrap();

    let slug = TenantSlug::new("preserve_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Preserve Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 10_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    // Snapshot what's there before re-applying.
    let before = l.tenant_settings(&slug).await.unwrap();
    let memberships_before = l.list_memberships_for("preserve@example.com").await.unwrap();

    // Re-apply — this is what gateway startup now does.
    l.migrate_all_tenants().await.expect("re-migrate");

    // Settings + memberships still there, untouched.
    let after = l.tenant_settings(&slug).await.unwrap();
    assert_eq!(after.display_name, before.display_name);
    assert_eq!(after.dues_amount_cents, before.dues_amount_cents);
    assert_eq!(after.cadence, before.cadence);

    let memberships_after = l.list_memberships_for("preserve@example.com").await.unwrap();
    assert_eq!(memberships_after.len(), memberships_before.len());
    assert_eq!(memberships_after[0].tenant_slug, slug.as_str());
}

#[tokio::test]
async fn create_tenant_and_membership() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "founder@example.com".into(),
            google_sub: "g_founder_1".into(),
            display_name: Some("Founder".into()),
        })
        .await
        .unwrap();

    let slug = TenantSlug::new("acme_capital").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Acme Capital".into(),
        timezone: "Asia/Kuala_Lumpur".into(),
        currency: "MYR".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 25_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    let memberships = l.list_memberships_for("founder@example.com").await.unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].tenant_slug, "acme_capital");
    assert_eq!(memberships[0].role, "owner");

    let settings = l.tenant_settings(&slug).await.unwrap();
    assert_eq!(settings.timezone, "Asia/Kuala_Lumpur");
    assert_eq!(settings.currency, "MYR");
    assert_eq!(settings.cadence, "monthly");
    assert_eq!(settings.dues_amount_cents, 25_000);
}

#[tokio::test]
async fn lock_rejects_past_period() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_2".into(),
            display_name: None,
        })
        .await
        .unwrap();

    let slug = TenantSlug::new("test_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Test Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 10_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    // Submitting for the *current* period must succeed.
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let current_period = format!("{:04}-{:02}", now.year(), now.month());
    l.add_contribution(
        &slug,
        NewContribution {
            user_email: "owner@example.com".into(),
            cadence: "monthly".into(),
            period: current_period.clone(),
            amount_cents: 10_000,
            proof_key: None,
            note: None,
        },
    )
    .await
    .expect("current period should succeed");

    // Submitting for last year must be rejected by the DB-side event.
    let past_period = "2020-01".to_string();
    let err = l
        .add_contribution(
            &slug,
            NewContribution {
                user_email: "owner@example.com".into(),
                cadence: "monthly".into(),
                period: past_period.clone(),
                amount_cents: 5_000,
                proof_key: None,
                note: None,
            },
        )
        .await
        .expect_err("past period must be rejected");

    match err {
        ledger::Error::PeriodLocked { period } => {
            assert_eq!(period, "2020-01");
        }
        other => panic!("expected PeriodLocked, got {other:?}"),
    }
}

#[tokio::test]
async fn weekly_cadence_lock_rejects_past_iso_week() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_3".into(),
            display_name: None,
        })
        .await
        .unwrap();

    let slug = TenantSlug::new("weekly_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Weekly Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "weekly".into(),
        dues_amount_cents: 2_500,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    // Current ISO week — must succeed.
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let iso = now.iso_week();
    let current_period = format!("{:04}-W{:02}", iso.year(), iso.week());
    l.add_contribution(
        &slug,
        NewContribution {
            user_email: "owner@example.com".into(),
            cadence: "weekly".into(),
            period: current_period,
            amount_cents: 2_500,
            proof_key: None,
            note: None,
        },
    )
    .await
    .expect("current ISO week should succeed");

    // Past ISO week — must be rejected.
    let err = l
        .add_contribution(
            &slug,
            NewContribution {
                user_email: "owner@example.com".into(),
                cadence: "weekly".into(),
                period: "2020-W01".into(),
                amount_cents: 2_500,
                proof_key: None,
                note: None,
            },
        )
        .await
        .expect_err("past ISO week must be rejected");
    assert!(matches!(err, ledger::Error::PeriodLocked { .. }));
}

// ─── Partial-payment + dues-cap tests ──────────────────────────────────────

/// Helper: build "YYYY-MM" for "right now", which is what the DB-side
/// lock event also uses (UTC).
fn current_monthly_period() -> String {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

#[tokio::test]
async fn partial_payments_sum_to_dues_and_summarise() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();
    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_partial".into(),
            display_name: None,
        })
        .await
        .unwrap();
    let slug = TenantSlug::new("partial_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Partial Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 10_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    let period = current_monthly_period();
    let mk = |amount: i64| NewContribution {
        user_email: "owner@example.com".into(),
        cadence: "monthly".into(),
        period: period.clone(),
        amount_cents: amount,
        proof_key: None,
        note: None,
    };

    // Two partials adding up to dues — both succeed and produce distinct rows.
    let a = l.add_contribution(&slug, mk(4_000)).await.unwrap();
    let b = l.add_contribution(&slug, mk(6_000)).await.unwrap();
    assert_ne!(a.id, b.id, "each payment is a separate row");

    let rows = l
        .list_contributions(&slug, "owner@example.com", &period)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);

    let s = l
        .period_summary(&slug, "owner@example.com", &period)
        .await
        .unwrap();
    assert_eq!(s.dues_cents, 10_000);
    assert_eq!(s.paid_cents, 10_000);
    assert_eq!(s.remaining_cents, 0);
}

#[tokio::test]
async fn dues_cap_rejects_overpayment() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();
    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_cap".into(),
            display_name: None,
        })
        .await
        .unwrap();
    let slug = TenantSlug::new("cap_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Cap Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 10_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    let period = current_monthly_period();
    let mk = |amount: i64| NewContribution {
        user_email: "owner@example.com".into(),
        cadence: "monthly".into(),
        period: period.clone(),
        amount_cents: amount,
        proof_key: None,
        note: None,
    };

    // First payment under the cap — succeeds.
    l.add_contribution(&slug, mk(7_000)).await.unwrap();
    // Second payment that would push the sum to 13_000 — rejected by event.
    let err = l
        .add_contribution(&slug, mk(6_000))
        .await
        .expect_err("over-cap payment must be rejected");
    match err {
        ledger::Error::DuesCapExceeded {
            paid_cents,
            dues_cents,
        } => {
            assert_eq!(paid_cents, 13_000);
            assert_eq!(dues_cents, 10_000);
        }
        other => panic!("expected DuesCapExceeded, got {other:?}"),
    }

    // The rejected payment must not have landed.
    let s = l
        .period_summary(&slug, "owner@example.com", &period)
        .await
        .unwrap();
    assert_eq!(s.paid_cents, 7_000);
    assert_eq!(s.remaining_cents, 3_000);
}

#[tokio::test]
async fn zero_dues_setting_disables_cap() {
    // When dues_amount_cents = 0, the cap is treated as "no cap" so members
    // can still record arbitrary contributions (e.g. one-off donations).
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();
    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_zero".into(),
            display_name: None,
        })
        .await
        .unwrap();
    let slug = TenantSlug::new("zero_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Zero Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 0,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    let period = current_monthly_period();
    l.add_contribution(
        &slug,
        NewContribution {
            user_email: "owner@example.com".into(),
            cadence: "monthly".into(),
            period: period.clone(),
            amount_cents: 999_999,
            proof_key: None,
            note: None,
        },
    )
    .await
    .expect("zero dues should not enforce a cap");

    let s = l
        .period_summary(&slug, "owner@example.com", &period)
        .await
        .unwrap();
    assert_eq!(s.dues_cents, 0);
    assert_eq!(s.paid_cents, 999_999);
    assert_eq!(s.remaining_cents, 0, "clamped at zero");
}

#[tokio::test]
async fn update_settings_patches_only_provided_fields() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "owner@example.com".into(),
            google_sub: "g_owner_4".into(),
            display_name: None,
        })
        .await
        .unwrap();

    let slug = TenantSlug::new("patchable_fund").unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Patchable Fund".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 5_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();

    // Empty patch — no-op, returns current state unchanged.
    let s = l
        .update_settings(&slug, UpdateSettings::default())
        .await
        .unwrap();
    assert_eq!(s.cadence, "monthly");
    assert_eq!(s.dues_amount_cents, 5_000);
    assert_eq!(s.display_name, "Patchable Fund");

    // Patch only cadence + dues — display_name + currency must survive.
    let s = l
        .update_settings(
            &slug,
            UpdateSettings {
                cadence: Some("weekly".into()),
                dues_amount_cents: Some(1_500),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.cadence, "weekly");
    assert_eq!(s.dues_amount_cents, 1_500);
    assert_eq!(s.display_name, "Patchable Fund");
    assert_eq!(s.currency, "USD");

    // Patch display_name only — cadence + dues survive.
    let s = l
        .update_settings(
            &slug,
            UpdateSettings {
                display_name: Some("Renamed Fund".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.display_name, "Renamed Fund");
    assert_eq!(s.cadence, "weekly");
    assert_eq!(s.dues_amount_cents, 1_500);
}

// ─── Invite flow tests ─────────────────────────────────────────────────────

/// Helper: spin up a ledger with one tenant + owner. Returns (ledger, owner_id, slug).
async fn fixture_with_tenant(slug_name: &str) -> (Ledger, ledger::RecordId, TenantSlug) {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();
    let owner = l
        .upsert_user(UpsertUser {
            email: "founder@example.com".into(),
            google_sub: format!("g_{slug_name}"),
            display_name: Some("Founder".into()),
        })
        .await
        .unwrap();
    let slug = TenantSlug::new(slug_name).unwrap();
    l.create_tenant(NewTenant {
        slug: slug.clone(),
        display_name: "Test Tenant".into(),
        timezone: "UTC".into(),
        currency: "USD".into(),
        cadence: "monthly".into(),
        dues_amount_cents: 1_000,
        created_by: owner.id.clone(),
    })
    .await
    .unwrap();
    (l, owner.id, slug)
}

/// Helper: pull the record-id key portion ("invite:abc" → "abc").
fn invite_key(rec: &ledger::InviteRecord) -> String {
    match &rec.id.key {
        ledger::RecordIdKey::String(s) => s.clone(),
        ledger::RecordIdKey::Uuid(u) => u.to_string(),
        ledger::RecordIdKey::Number(n) => n.to_string(),
        _ => panic!("unexpected invite id key shape"),
    }
}

#[tokio::test]
async fn revoke_marks_invite_revoked_and_blocks_double_revoke() {
    let (l, owner_id, slug) = fixture_with_tenant("revoke_target").await;

    let inv = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .expect("create invite");
    let key = invite_key(&inv);
    assert!(inv.revoked_at.is_none());
    assert!(inv.accepted_at.is_none());

    // First revoke succeeds, sets revoked_at.
    let r = l.revoke_invite(slug.as_str(), &key).await.expect("revoke");
    assert!(r.revoked_at.is_some(), "revoked_at should be set");
    assert_eq!(r.tenant_slug, slug.as_str());

    // Second revoke must NotFound — the WHERE filter excludes already-revoked rows.
    let err = l
        .revoke_invite(slug.as_str(), &key)
        .await
        .expect_err("second revoke must fail");
    assert!(matches!(err, ledger::Error::NotFound), "got {err:?}");
}

#[tokio::test]
async fn reinvite_after_revoke_reactivates_same_row() {
    let (l, owner_id, slug) = fixture_with_tenant("reactivate_fund").await;

    let first = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .unwrap();
    l.revoke_invite(slug.as_str(), &invite_key(&first))
        .await
        .unwrap();

    // Re-invite the same email with a different role — should reactivate
    // the existing row, not create a new one (unique index would block CREATE).
    let second = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "treasurer".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .expect("re-invite");

    assert_eq!(
        invite_key(&first),
        invite_key(&second),
        "same row reactivated"
    );
    assert!(second.revoked_at.is_none());
    assert_eq!(second.role, "treasurer");

    // List should show exactly one invite.
    let list = l.list_invites_for_tenant(slug.as_str()).await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn duplicate_active_invite_is_rejected() {
    let (l, owner_id, slug) = fixture_with_tenant("dup_fund").await;

    l.create_invite(NewInvite {
        tenant_slug: slug.as_str().into(),
        email: "guest@example.com".into(),
        role: "member".into(),
        invited_by: owner_id.clone(),
    })
    .await
    .unwrap();

    let err = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .expect_err("duplicate active invite must fail");
    assert!(
        matches!(err, ledger::Error::InviteExists { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn delete_pending_invite_removes_row() {
    let (l, owner_id, slug) = fixture_with_tenant("del_pending").await;

    let inv = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .unwrap();

    l.delete_invite(slug.as_str(), &invite_key(&inv))
        .await
        .expect("delete pending");

    let list = l.list_invites_for_tenant(slug.as_str()).await.unwrap();
    assert!(list.is_empty(), "row should be gone");
}

#[tokio::test]
async fn delete_revoked_invite_removes_row() {
    let (l, owner_id, slug) = fixture_with_tenant("del_revoked").await;

    let inv = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .unwrap();
    let key = invite_key(&inv);
    l.revoke_invite(slug.as_str(), &key).await.unwrap();

    // Delete still works on a revoked row (only accepted ones are protected).
    l.delete_invite(slug.as_str(), &key)
        .await
        .expect("delete revoked");

    let list = l.list_invites_for_tenant(slug.as_str()).await.unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn delete_accepted_invite_is_blocked() {
    let (l, owner_id, slug) = fixture_with_tenant("del_accepted").await;

    let inv = l
        .create_invite(NewInvite {
            tenant_slug: slug.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner_id.clone(),
        })
        .await
        .unwrap();
    let key = invite_key(&inv);

    // Have the invitee sign in, materializing the invite into a membership.
    let invitee = l
        .upsert_user(UpsertUser {
            email: "guest@example.com".into(),
            google_sub: "g_guest_1".into(),
            display_name: Some("Guest".into()),
        })
        .await
        .unwrap();
    let accepted = l
        .accept_pending_invites("guest@example.com", invitee.id.clone())
        .await
        .unwrap();
    assert_eq!(accepted, vec![slug.as_str().to_string()]);

    // Hard-delete must refuse — the membership record depends on this audit row.
    let err = l
        .delete_invite(slug.as_str(), &key)
        .await
        .expect_err("delete on accepted must fail");
    assert!(matches!(err, ledger::Error::NotFound), "got {err:?}");

    // Invite still listed; status is "accepted".
    let list = l.list_invites_for_tenant(slug.as_str()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].accepted_at.is_some());
}

#[tokio::test]
async fn delete_in_wrong_tenant_is_blocked() {
    let l = Ledger::connect(cfg()).await.expect("connect");
    l.apply_control_schema().await.unwrap();

    let owner = l
        .upsert_user(UpsertUser {
            email: "founder@example.com".into(),
            google_sub: "g_cross_owner".into(),
            display_name: None,
        })
        .await
        .unwrap();
    let slug_a = TenantSlug::new("tenant_alpha").unwrap();
    let slug_b = TenantSlug::new("tenant_beta").unwrap();
    for s in [&slug_a, &slug_b] {
        l.create_tenant(NewTenant {
            slug: s.clone(),
            display_name: "T".into(),
            timezone: "UTC".into(),
            currency: "USD".into(),
            cadence: "monthly".into(),
            dues_amount_cents: 0,
            created_by: owner.id.clone(),
        })
        .await
        .unwrap();
    }

    let inv = l
        .create_invite(NewInvite {
            tenant_slug: slug_a.as_str().into(),
            email: "guest@example.com".into(),
            role: "member".into(),
            invited_by: owner.id.clone(),
        })
        .await
        .unwrap();
    let key = invite_key(&inv);

    // Try deleting tenant_alpha's invite via tenant_beta — must fail.
    let err = l
        .delete_invite(slug_b.as_str(), &key)
        .await
        .expect_err("cross-tenant delete must fail");
    assert!(matches!(err, ledger::Error::NotFound), "got {err:?}");

    // The invite is still alive in tenant_alpha.
    let list = l.list_invites_for_tenant(slug_a.as_str()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].revoked_at.is_none());
}

#[tokio::test]
async fn accept_pending_invites_creates_membership_and_marks_accepted() {
    let (l, owner_id, slug) = fixture_with_tenant("acc_test").await;

    l.create_invite(NewInvite {
        tenant_slug: slug.as_str().into(),
        email: "guest@example.com".into(),
        role: "treasurer".into(),
        invited_by: owner_id.clone(),
    })
    .await
    .unwrap();

    let invitee = l
        .upsert_user(UpsertUser {
            email: "guest@example.com".into(),
            google_sub: "g_acc_guest".into(),
            display_name: Some("Guest".into()),
        })
        .await
        .unwrap();

    let accepted = l
        .accept_pending_invites("guest@example.com", invitee.id.clone())
        .await
        .unwrap();
    assert_eq!(accepted, vec![slug.as_str().to_string()]);

    // Membership materialized with the invited role.
    let memberships = l.list_memberships_for("guest@example.com").await.unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].tenant_slug, slug.as_str());
    assert_eq!(memberships[0].role, "treasurer");

    // The members listing returns owner + invitee.
    let members = l.list_tenant_members(slug.as_str()).await.unwrap();
    assert_eq!(members.len(), 2);
    let emails: Vec<_> = members.iter().map(|m| m.email.as_str()).collect();
    assert!(emails.contains(&"founder@example.com"));
    assert!(emails.contains(&"guest@example.com"));

    // Re-running on the next sign-in is idempotent: no new accepts, no
    // duplicate membership.
    let again = l
        .accept_pending_invites("guest@example.com", invitee.id.clone())
        .await
        .unwrap();
    assert!(again.is_empty(), "no new invites the second time");
    let memberships = l.list_memberships_for("guest@example.com").await.unwrap();
    assert_eq!(memberships.len(), 1);
}
