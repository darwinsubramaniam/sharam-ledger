-- Sharam per-tenant schema (SurrealDB 3.x)
-- Apply with:  USE NS <tenant_slug> DB main;  THEN this file.
--
-- The namespace IS the tenant boundary. No row in here carries a tenant FK
-- — by virtue of being in this namespace, every record belongs to this
-- tenant and only this tenant.

-- ─── settings:current ───────────────────────────────────────────────────────
-- Singleton row. The lock invariant reads `timezone` from here to compute
-- the tenant's current period. `cadence` + `dues_amount_cents` define how
-- often members owe and how much per cycle. Both are mutable.
DEFINE TABLE OVERWRITE settings SCHEMAFULL;
DEFINE FIELD OVERWRITE timezone          ON settings TYPE string DEFAULT "UTC";
DEFINE FIELD OVERWRITE currency          ON settings TYPE string DEFAULT "USD";
DEFINE FIELD OVERWRITE display_name      ON settings TYPE string;
DEFINE FIELD OVERWRITE cadence           ON settings TYPE string
    DEFAULT "monthly"
    ASSERT $value INSIDE ["weekly", "monthly", "yearly"];
DEFINE FIELD OVERWRITE dues_amount_cents ON settings TYPE int DEFAULT 0 ASSERT $value >= 0;
DEFINE FIELD OVERWRITE purpose           ON settings TYPE option<string>
    ASSERT $value = NONE OR string::len($value) <= 600;
DEFINE FIELD OVERWRITE created_at        ON settings TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at        ON settings TYPE datetime DEFAULT time::now() VALUE time::now();

-- ─── helpers ────────────────────────────────────────────────────────────────
-- Compute the current period in the given cadence. UTC-based — SurrealDB
-- 3.x dropped the tz arg from time::format. The app layer refines this
-- using chrono-tz against `settings:current.timezone`; this DB-side check
-- is a coarser defense-in-depth guard.
--
-- Format strings (chrono specifiers):
--   weekly  → "%G-W%V"  (ISO 8601 week-year + ISO week, e.g. "2026-W18")
--   monthly → "%Y-%m"   (e.g. "2026-05")
--   yearly  → "%Y"      (e.g. "2026")
DEFINE FUNCTION OVERWRITE fn::current_period_for($cadence: string) {
    IF $cadence = "weekly" {
        RETURN time::format(time::now(), "%G-W%V");
    } ELSE IF $cadence = "yearly" {
        RETURN time::format(time::now(), "%Y");
    } ELSE {
        RETURN time::format(time::now(), "%Y-%m");
    };
};

-- ─── contribution ───────────────────────────────────────────────────────────
-- A single payment from a member towards their dues for `period`. Many rows
-- per (user_email, period) — partial payments accumulate. The DB-side dues
-- cap (below) enforces that the non-rejected sum for any (user, period) does
-- not exceed `settings:current.dues_amount_cents`.
--
-- `period` is one of:
--   weekly  "YYYY-Www"   (ISO 8601 week)
--   monthly "YYYY-MM"
--   yearly  "YYYY"
-- in tenant-local time. `cadence` is snapshotted from settings at write-time
-- so historical rows survive cadence changes. Once
-- `period < fn::current_period_for(cadence)`, the row is immutable.
DEFINE TABLE OVERWRITE contribution SCHEMAFULL;
DEFINE FIELD OVERWRITE user_email   ON contribution TYPE string
    ASSERT string::is_email($value);
DEFINE FIELD OVERWRITE cadence      ON contribution TYPE string
    ASSERT $value INSIDE ["weekly", "monthly", "yearly"];
DEFINE FIELD OVERWRITE period       ON contribution TYPE string
    ASSERT string::matches($value, "^[0-9]{4}-W(0[1-9]|[1-4][0-9]|5[0-3])$")
        OR string::matches($value, "^[0-9]{4}-(0[1-9]|1[0-2])$")
        OR string::matches($value, "^[0-9]{4}$");
DEFINE FIELD OVERWRITE amount_cents ON contribution TYPE int ASSERT $value > 0;
DEFINE FIELD OVERWRITE status       ON contribution TYPE string
    DEFAULT "submitted"
    ASSERT $value INSIDE ["submitted", "verified", "rejected"];
DEFINE FIELD OVERWRITE proof_key    ON contribution TYPE option<string>;
DEFINE FIELD OVERWRITE note         ON contribution TYPE option<string>;
DEFINE FIELD OVERWRITE submitted_at ON contribution TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at   ON contribution TYPE datetime DEFAULT time::now() VALUE time::now();
DEFINE FIELD OVERWRITE locked_at    ON contribution TYPE option<datetime>;

-- Lookup index for "this user's payments for this period". Non-unique —
-- the cap is enforced by the dues_cap event below, not by uniqueness.
DEFINE INDEX OVERWRITE contribution_user_period_idx
    ON contribution FIELDS user_email, period;

-- DB-side lock invariant: rejects writes whose period is strictly older
-- than the current period for that row's cadence. Even a buggy/replayed
-- app request can't slip past this. Comparison is lexicographic — safe
-- within a single cadence because all formats are zero-padded.
DEFINE EVENT OVERWRITE contribution_lock_check ON TABLE contribution
    WHEN $event = "CREATE" OR $event = "UPDATE"
    THEN {
        LET $cur = fn::current_period_for($after.cadence);
        IF $after.period < $cur {
            THROW "period_locked: " + $after.period + " < " + $cur;
        };
    };

-- DB-side dues cap: the non-rejected sum of payments for a given
-- (user_email, period) MUST NOT exceed settings:current.dues_amount_cents.
-- Sums the table AFTER the write, so the new/updated row is included.
-- Rejected rows are excluded so a treasurer can void a wrong submission
-- and re-collect without re-tripping the cap.
DEFINE EVENT OVERWRITE contribution_dues_cap ON TABLE contribution
    WHEN $event = "CREATE" OR $event = "UPDATE"
    THEN {
        IF $after.status = "rejected" {
            RETURN NONE;
        };
        LET $dues = (SELECT VALUE dues_amount_cents FROM ONLY settings:current);
        LET $paid = math::sum(
            SELECT VALUE amount_cents FROM contribution
            WHERE user_email = $after.user_email
              AND period      = $after.period
              AND status     != "rejected"
        );
        IF $dues > 0 AND $paid > $dues {
            THROW "dues_cap_exceeded: " + <string> $paid + " > " + <string> $dues;
        };
    };

DEFINE EVENT OVERWRITE contribution_delete_lock ON TABLE contribution
    WHEN $event = "DELETE"
    THEN {
        LET $cur = fn::current_period_for($before.cadence);
        IF $before.period < $cur {
            THROW "period_locked: cannot delete locked contribution " + $before.period;
        };
    };

-- ─── carry_forward ──────────────────────────────────────────────────────────
-- Singleton row capturing money the venture had collected BEFORE migrating
-- to Sharam. Set ONCE by the owner at venture creation; immutable afterwards.
-- Treats the period-lock / dues-cap invariants as orthogonal — carry-forward
-- is treasury seed, not a member dues row, so no per-member or per-period
-- bookkeeping applies. Use id `carry_forward:current`.
--
-- `from_date` / `to_date` are ISO calendar dates (YYYY-MM-DD) describing the
-- window over which the seed amount was accumulated off-platform. Stored as
-- strings so there's no tz ambiguity for an inherently fuzzy "this date
-- range, roughly".
DEFINE TABLE OVERWRITE carry_forward SCHEMAFULL;
DEFINE FIELD OVERWRITE from_date    ON carry_forward TYPE string
    ASSERT string::matches($value, "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$");
DEFINE FIELD OVERWRITE to_date      ON carry_forward TYPE string
    ASSERT string::matches($value, "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$");
DEFINE FIELD OVERWRITE amount_cents ON carry_forward TYPE int ASSERT $value > 0;
DEFINE FIELD OVERWRITE note         ON carry_forward TYPE option<string>;
DEFINE FIELD OVERWRITE recorded_by  ON carry_forward TYPE string
    ASSERT string::is_email($value);
DEFINE FIELD OVERWRITE recorded_at  ON carry_forward TYPE datetime
    DEFAULT time::now() READONLY;

-- Write-once: reject any UPDATE/DELETE so once seeded, the row is permanent.
-- CREATE on an already-existing id is rejected by SurrealDB itself, so the
-- "row exists" backstop is the duplicate-id error — the app surfaces that
-- as Error::CarryForwardExists.
DEFINE EVENT OVERWRITE carry_forward_immutable ON TABLE carry_forward
    WHEN $event = "UPDATE" OR $event = "DELETE"
    THEN {
        THROW "carry_forward_immutable: the carry-forward seed is set once at venture creation and cannot be modified";
    };

-- ─── audit_log ──────────────────────────────────────────────────────────────
DEFINE TABLE OVERWRITE audit_log SCHEMAFULL;
DEFINE FIELD OVERWRITE actor_email ON audit_log TYPE option<string>;
DEFINE FIELD OVERWRITE action      ON audit_log TYPE string;
DEFINE FIELD OVERWRITE target      ON audit_log TYPE option<string>;
DEFINE FIELD OVERWRITE detail      ON audit_log TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE at          ON audit_log TYPE datetime DEFAULT time::now() READONLY;
DEFINE INDEX OVERWRITE audit_at ON audit_log FIELDS at;
