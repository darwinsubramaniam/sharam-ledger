-- Sharam control plane schema (SurrealDB 3.x)
-- Apply with:  USE NS system DB control;  THEN this file.
--
-- Holds the data that must be queryable BEFORE we know which tenant the
-- request is for: Google identities, invites, and which slugs each user
-- can access. Tenant-scoped data lives in per-tenant namespaces and is
-- governed by `schema/tenant/000_init.surql`.

-- ─── user ───────────────────────────────────────────────────────────────────
-- A Sharam identity. `email` is the natural lookup key. Sign-in methods are
-- additive on the same row: a user may have `google_sub` set (Google
-- sign-in), `password_hash` set (email/password), or both (account merged
-- on second sign-in via the same email). At least one of the two must be
-- present in practice; the schema doesn't enforce this so the row can be
-- created with one method and have the other added later.
DEFINE TABLE OVERWRITE user SCHEMAFULL;
DEFINE FIELD OVERWRITE email         ON user TYPE string ASSERT string::is_email($value);
DEFINE FIELD OVERWRITE google_sub    ON user TYPE option<string>;
-- PHC-format Argon2id hash. NEVER returned over the wire.
DEFINE FIELD OVERWRITE password_hash ON user TYPE option<string>;
DEFINE FIELD OVERWRITE display_name  ON user TYPE option<string>;
DEFINE FIELD OVERWRITE display_tz    ON user TYPE string DEFAULT "UTC";
DEFINE FIELD OVERWRITE created_at    ON user TYPE datetime DEFAULT time::now() READONLY;
DEFINE INDEX OVERWRITE user_email_uniq ON user FIELDS email UNIQUE;
-- google_sub uniqueness was previously enforced by a UNIQUE index, but
-- SurrealDB treats NONE as a value so multiple password-only users would
-- collide on it. Drop the unique constraint and keep a plain lookup index;
-- de-facto uniqueness still holds because the only writer (`upsert_user`)
-- keys by email and email is unique.
REMOVE INDEX IF EXISTS user_google_sub_uniq ON TABLE user;
DEFINE INDEX OVERWRITE user_google_sub_idx ON user FIELDS google_sub;

-- ─── invite ─────────────────────────────────────────────────────────────────
-- Gates which Google emails may join which tenant. `tenant_slug` is the
-- target SurrealDB namespace name; we deliberately do NOT use record<tenant>
-- because there is no tenant *table* — the namespace itself is the tenant.
DEFINE TABLE OVERWRITE invite SCHEMAFULL;
DEFINE FIELD OVERWRITE tenant_slug ON invite TYPE string
    ASSERT string::matches($value, "^[a-z][a-z0-9_]{2,40}$");
DEFINE FIELD OVERWRITE email       ON invite TYPE string ASSERT string::is_email($value);
DEFINE FIELD OVERWRITE role        ON invite TYPE string
    ASSERT $value INSIDE ["owner", "treasurer", "member"];
DEFINE FIELD OVERWRITE invited_by  ON invite TYPE record<user>;
DEFINE FIELD OVERWRITE created_at  ON invite TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE accepted_at ON invite TYPE option<datetime>;
DEFINE FIELD OVERWRITE revoked_at  ON invite TYPE option<datetime>;
DEFINE INDEX OVERWRITE invite_tenant_email_uniq ON invite FIELDS tenant_slug, email UNIQUE;

-- ─── membership ─────────────────────────────────────────────────────────────
-- Join table: which user can access which tenant slug, and in what role.
-- The gateway uses this to populate the tenant picker after login.
DEFINE TABLE OVERWRITE membership SCHEMAFULL;
DEFINE FIELD OVERWRITE user        ON membership TYPE record<user>;
DEFINE FIELD OVERWRITE tenant_slug ON membership TYPE string
    ASSERT string::matches($value, "^[a-z][a-z0-9_]{2,40}$");
DEFINE FIELD OVERWRITE role        ON membership TYPE string
    ASSERT $value INSIDE ["owner", "treasurer", "member"];
DEFINE FIELD OVERWRITE created_at  ON membership TYPE datetime DEFAULT time::now() READONLY;
DEFINE INDEX OVERWRITE membership_user_tenant_uniq ON membership FIELDS user, tenant_slug UNIQUE;
DEFINE INDEX OVERWRITE membership_tenant_lookup    ON membership FIELDS tenant_slug;

-- ─── tenant_directory (optional metadata cache) ─────────────────────────────
-- Lightweight registry for "list all tenants" admin views. Not required for
-- correctness — it's just an index. Per-tenant truth still lives in the
-- tenant's own `settings:current` row.
DEFINE TABLE OVERWRITE tenant_directory SCHEMAFULL;
DEFINE FIELD OVERWRITE slug         ON tenant_directory TYPE string
    ASSERT string::matches($value, "^[a-z][a-z0-9_]{2,40}$");
DEFINE FIELD OVERWRITE display_name ON tenant_directory TYPE string;
DEFINE FIELD OVERWRITE created_by   ON tenant_directory TYPE record<user>;
DEFINE FIELD OVERWRITE created_at   ON tenant_directory TYPE datetime DEFAULT time::now() READONLY;
DEFINE INDEX OVERWRITE tenant_directory_slug_uniq ON tenant_directory FIELDS slug UNIQUE;
