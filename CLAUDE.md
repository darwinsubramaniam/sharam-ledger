# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Cargo workspace, edition 2024, resolver 3. All commands run from the repo root.

- Build everything: `cargo build`
- Run the HTTP server: `cargo run -p gateway`
- Run the frontend dev server: `dx serve --package sharam-ui` (from repo root)
- Test all crates: `cargo test`
- Test one crate: `cargo test -p ledger` (or `auth`, `common`)
- Run a single test: `cargo test -p ledger create_tenant_and_membership`
- Lint: `cargo clippy --workspace --all-targets`
- Format: `cargo fmt --all`

`ledger` tests use SurrealDB in-memory mode (`kv-mem` dev-feature, endpoint `"memory"`) — no external services required.

## Configuration

Config is loaded by `common::config::AppConfig::load()` via figment, in this precedence order:

1. `Sharam.toml` at the repo root (copy from `Sharam.example.toml`; gitignored).
2. `SHARAM_*` env vars, with `__` as the nesting separator (e.g. `SHARAM_GATEWAY__BIND=0.0.0.0:8080`, `SHARAM_GOOGLE__CLIENT_ID=...`).
3. A `.env` file is auto-loaded if present.

Required sections: `[gateway]`, `[surrealdb]`, `[storage]` (RustFS, S3-compatible), `[google]`.

## Docker / Compose

`compose.yml` at the repo root is the single source of truth for both local dev and personal-server deploys. It uses a profile to switch modes:

- **Dev** — `docker compose up -d` brings up only `surrealdb` + `rustfs` (host ports `127.0.0.1:8000`/`9000`). Run `cargo run -p gateway` and `dx serve --package sharam-ui` on the host for HMR. The `Sharam.toml` on disk is what figment reads.
- **Server / full stack** — `docker compose --profile app up -d --build` adds `gateway` (the axum binary) and `web` (Caddy serving the Dioxus dist + reverse-proxying `/api/*` → `gateway:8080`). Containers do **not** read `Sharam.toml`; figment falls through to `SHARAM_*` env vars injected by compose.

### File map

- `compose.yml` — service definitions + profiles
- `docker/gateway.Dockerfile` — cargo-chef multi-stage. Scoped to `-p gateway` because `sharam-ui` is a workspace member with `default = ["web"]` (wasm-only); a workspace-wide cook would try to compile dioxus/web for the host target and fail.
- `docker/web.Dockerfile` — three stages: Tailwind v4 CSS compile (Node) → `dx bundle --release --platform web` (Rust + dx CLI) → Caddy. The Google client ID is threaded through as a build arg `SHARAM_GOOGLE_CLIENT_ID` so `sharam-ui/build.rs` embeds it into the wasm at compile time.
- `docker/Caddyfile` — `{$SITE_ADDRESS:":80"}` site block; defaults to plain HTTP on `:80`, but pointing `SITE_ADDRESS` at a real domain makes Caddy auto-provision Let's Encrypt.
- `.env.example` — copy to `.env`. Required: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REDIRECT_URI`. Compose interpolation uses `:?` so the build refuses to start without them.
- `.dockerignore` — keeps `target/`, `Sharam.toml`, `.env`, and `**/node_modules/` out of the build context.

### Watchpoints

- The web image expects the `dx bundle` web output at `target/dx/sharam-ui/release/web/public`. If you upgrade `dioxus-cli` and the path changes, fix the `COPY --from=dx-builder` in `docker/web.Dockerfile`.
- `compose.yml` pins `surrealdb/surrealdb:latest` and `rustfs/rustfs:latest` — pin to specific versions before treating any deploy as production. The SurrealDB healthcheck shells out to `/surreal is-ready`; if that flag spelling changes between releases, swap for a TCP probe.
- The `web` build arg `SHARAM_GOOGLE_CLIENT_ID` and the `gateway` env var `SHARAM_GOOGLE__CLIENT_ID` must be the same value — they're both derived from `${GOOGLE_CLIENT_ID}` in compose, so set it once in `.env`.

## Architecture

Rust workspace with a backend (five crates under `crate/`) and a Dioxus frontend (`sharam-ui/`). All members share pinned versions via `[workspace.dependencies]` in the root `Cargo.toml` — crate manifests should reference deps as `{ workspace = true }` rather than re-pinning.

- **`common`** — `AppConfig` (figment), tracing init, shared error type, and the domain primitives every other crate depends on: `TenantSlug`, `Period` (YYYY-MM, tenant-tz aware via `chrono-tz`), `Role`, `ContributionStatus`, and UUIDv7 ID newtypes (`UserId`, `MembershipId`, …).
- **`auth`** — `GoogleVerifier` for Google-issued ID tokens: RS256, JWKS fetched from Google's well-known endpoint and cached with a 1-hour TTL behind an `RwLock`. `with_static_keys()` is the test seam — sign + verify locally without hitting Google.
- **`ledger`** — SurrealDB 3.x client. Owns one persistent control-plane connection and lazily opens one cached connection per tenant. Schemas live in `crate/ledger/schema/{control,tenant}/*.surql` and are embedded with `include_str!`; apply with `Ledger::apply_control_schema()` and (per-tenant) `Ledger::create_tenant()`. Read methods include `upsert_user`, `list_memberships_for(email)`, `list_user_ventures(email)` (joins `membership` + `tenant_directory` for the dashboard view), and the contribution surface: `add_contribution` (CREATE — each call is one payment row), `list_contributions(slug, email, period)`, and `period_summary(slug, email, period) -> PeriodSummary { dues_cents, paid_cents, remaining_cents }`.
- **`storage`** — scaffold for RustFS via `aws-sdk-s3` (S3-compatible). Currently exports error types only.
- **`gateway`** — axum 0.8 HTTP entry point (`crate/gateway/src/main.rs`). Routes mounted under `crate/gateway/src/routes/`. `AppState` carries `Arc<GoogleVerifier>` + `Ledger` (control schema is applied at startup) + `Mailer`. CORS bound to `gateway.frontend_origin`. Routes today:
  - `GET /health`
  - `POST /api/auth/google` (verifies a credential, returns user info)
  - `POST /api/tenants` (creates tenant + first owner-membership)
  - `GET /api/me/ventures` (caller's tenants joined with directory display name + role)
  - `GET|PATCH /api/tenants/:slug/settings` (read by any member; PATCH owner-only)
  - `GET /api/tenants/:slug/members`
  - `GET|POST /api/tenants/:slug/invites` + `DELETE /api/tenants/:slug/invites/:key` + `POST .../revoke`
  - `POST /api/tenants/:slug/contributions` — caller appends a payment for the current period. Body `{ amount_cents, note?, proof_key? }`. Period and cadence are derived from `settings:current` server-side; the client never picks the period.
  - `GET /api/tenants/:slug/contributions/me?period=…` — caller's payments + roll-up for `period` (defaults to current). Returns `{ summary: { period, cadence, currency, dues_cents, paid_cents, remaining_cents }, contributions: [...] }`.
- **`sharam-ui`** — Dioxus 0.7 frontend (web/desktop/server features) styled with Tailwind. Lives at the workspace root rather than under `crate/`. Entry point `sharam-ui/src/main.rs`, config in `sharam-ui/Dioxus.toml`, styles in `sharam-ui/tailwind.css`. See `sharam-ui/CLAUDE.md` for Dioxus 0.7 API conventions (signals, `use_resource`, `#[component]`, `Routable`) — Dioxus 0.7 dropped `cx`, `Scope`, and `use_state`, so do not use those.

### Multi-tenancy model

There is **no `tenant` table**. Each tenant is a SurrealDB namespace whose name is its `TenantSlug` (regex `^[a-z][a-z0-9_]{2,40}$`).

- **Control plane** (configured ns/db, e.g. `system/control`): `user`, `membership`, `invite`, `tenant_directory`. This is what's queryable before you know which tenant a request belongs to.
- **Per-tenant plane** (`ns=<slug> db=main`): `settings:current` (singleton with `timezone`/`currency`), `contribution`, `audit_log`. By virtue of namespace isolation, no row carries a tenant FK.

`TenantSlug::new` validation MUST stay in sync with the regex used in the SurrealDB schema — the same string is interpolated as a SurrealDB namespace name.

### Contribution invariants (period lock + dues cap)

The `contribution` table carries **two DB-side `EVENT`s** in `schema/tenant/000_init.surql` that the app cannot bypass:

1. **`contribution_lock_check`** + **`contribution_delete_lock`** — reject CREATE/UPDATE/DELETE when `period < fn::current_period_for(cadence)`. SurrealDB 3.x dropped the tz arg from `time::format`, so this check is **UTC-only** — coarser than tenant-local time, but a hard backstop. The app layer refines it with `chrono-tz` against `settings:current.timezone` (`Period::current_in(tz, cadence)` in `common::domain`). DB throws map to `ledger::Error::PeriodLocked { period }`.
2. **`contribution_dues_cap`** — sums non-rejected `amount_cents` for `(user_email, period)` AFTER the write and rejects the row if the total exceeds `settings:current.dues_amount_cents`. `dues_amount_cents = 0` means "no cap" (donation-style ventures). Rejected rows are excluded from the sum so a treasurer can void a wrong submission and the member can re-submit. DB throws map to `ledger::Error::DuesCapExceeded { paid_cents, dues_cents }`.

Both throws are parsed in `ledger::error::map_db_error` and surface as `422 Unprocessable Entity` from the gateway with the `{ok:false, error:"…"}` envelope.

The cap is what allows partial payments: many `contribution` rows per `(user, period)` whose non-rejected sum is bounded above by the dues amount.

### Auth model on the wire

Mutating routes and `/api/me/*` expect `Authorization: Bearer <google_id_token>`. There is **no session token** — the frontend keeps the original Google ID JWT in `localStorage["sharam_id_token"]` after `/api/auth/google` succeeds, and attaches it on every subsequent request. The gateway re-verifies via `GoogleVerifier::verify` on each call. Tokens expire after ~1h; expired tokens cause 401 and the UI prompts re-sign-in. When introducing a real session, replace at the gateway and frontend in lockstep.

### Adding a gateway route

1. New module under `crate/gateway/src/routes/`, return a `Router<AppState>` from `pub fn router()`.
2. Auth-protected routes pull `Authorization: Bearer …` from `HeaderMap`, then `state.google.verify(token).await` to get `GoogleClaims`. Use `claims.email` as the join key into `user`/`membership`.
3. Map `ledger::Error` to HTTP status: `TenantExists` → 409, `InviteExists` → 409, `PeriodLocked` → 422, `DuesCapExceeded` → 422, `NotFound` → 404, anything else → 500. Return `{ok:false, error:"..."}` envelope on errors so the frontend can surface a message.
4. Mount in `routes/mod.rs` and merge in `main::build_app`.

### Other conventions to preserve

- `contribution` rows are append-only payments: each call to `Ledger::add_contribution` `CREATE`s a new row with an auto-assigned id. There is **no** `(user_email, period)` unique index — the dues-cap event enforces the per-cycle bound instead. The `contribution_user_period_idx` index exists only to speed up the cap sum and the `list_contributions` lookup.
- `Ledger::connect_to` skips signin for embedded SurrealDB endpoints (`memory`, `file://`, etc.) — only ws/wss/http/https endpoints get `Root` signin. This is what lets the in-memory smoke tests work.
- Schema files use `DEFINE … OVERWRITE` everywhere so re-applying is safe (idempotent migrations).
- `MembershipRecord.role` is stored as a `String` to keep the `ledger` crate from having to teach `SurrealValue` about `common::domain::Role`. Map at the wire boundary in `gateway`, not in `ledger`.
- `ContributionRecord.id` is a `surrealdb::types::RecordId` with no `Display` impl. To render at the wire, match on `id.key` (`RecordIdKey::String|Uuid|Number`) and stringify the key portion — see `contribution_view` in `crate/gateway/src/routes/contributions.rs`.
