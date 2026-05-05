# Sharam

A self-hostable ledger for invite-only ventures — SMEs, investor groups,
co-ops, or any small group pooling money toward a shared goal. Members track
periodic dues, attach payment receipts, and view per-venture P&L.

> **Status:** early-stage. APIs and schemas may change without migration paths
> until v1.0.

## Stack

- **Backend** — Rust workspace (edition 2024). Five crates under `crate/`:
  `gateway` (axum 0.8 HTTP), `auth` (Google OIDC + Argon2id sessions),
  `ledger` (SurrealDB 3.x client), `storage` (S3-compatible via aws-sdk),
  `common` (config, domain primitives).
- **Frontend** — [Dioxus](https://dioxuslabs.com/) 0.7 + Tailwind v4
  (`sharam-ui/`).
- **Datastore** — [SurrealDB](https://surrealdb.com/) 3.x, namespace-per-tenant.
- **Object storage** — [RustFS](https://github.com/rustfs/rustfs)
  (S3-compatible) for receipt proofs.
- **Auth** — Google Sign-In *or* email + password. Both mint a single HS256
  session JWT.

## Architecture highlights

- **Tenant isolation by namespace.** There is no `tenant` table — each
  venture is a SurrealDB namespace whose name is its slug. Per-tenant data
  carries no tenant FK by construction.
- **Append-only contributions with DB-side invariants.** Each payment is its
  own row. Two SurrealDB `EVENT`s enforce a period lock (no writes to past
  cycles) and a dues cap (sum of non-rejected payments per `(member, period)`
  cannot exceed the venture's configured dues amount). The app layer cannot
  bypass these.
- **Stateless sessions.** Verification is a local HMAC check; no per-request
  network hop. Rotating `gateway.session_secret` invalidates every live
  session at once.

See `CLAUDE.md` for the full developer-facing architecture notes.

## Quick start (development)

Prerequisites: Rust (`rustup`, edition 2024 toolchain), Node.js 20+,
[`dioxus-cli`](https://dioxuslabs.com/learn/0.7/getting_started/) (`cargo
install dioxus-cli`), Docker.

```sh
# 1. Bring up SurrealDB + RustFS
docker compose up -d

# 2. Configure
cp Sharam.example.toml Sharam.toml
# Edit Sharam.toml — set session_secret (32+ random bytes) and Google OAuth creds
#   openssl rand -base64 48     # generate session_secret

# 3. Run the gateway (terminal 1)
cargo run -p gateway

# 4. Run the UI dev server (terminal 2)
dx serve --package sharam-ui
```

Open <http://localhost:8080> for the UI; the gateway listens on `:3000`.

## Tests

```sh
cargo test                                  # all crates
cargo test -p ledger                        # one crate
cargo test -p ledger create_tenant          # one test
cargo clippy --workspace --all-targets
cargo fmt --all
```

`ledger` tests use SurrealDB in-memory mode — no external services needed.

## Configuration

`common::config::AppConfig::load()` reads, in order:

1. `Sharam.toml` at the repo root (gitignored).
2. `SHARAM_*` env vars, with `__` as the nesting separator (e.g.
   `SHARAM_GATEWAY__BIND=0.0.0.0:8080`,
   `SHARAM_GOOGLE__CLIENT_ID=...`).
3. A `.env` file is auto-loaded if present.

See `Sharam.example.toml` for the schema and `.env.example` for env-var form.

## Production / self-hosting

The full stack runs from a single `compose.yml` with the `app` profile:

```sh
cp .env.example .env
# Set GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, GOOGLE_REDIRECT_URI, SITE_ADDRESS
docker compose --profile app up -d --build
```

This adds:

- `gateway` — axum binary (built via cargo-chef).
- `web` — Caddy fronting the Dioxus SPA, reverse-proxying `/api/*` to
  `gateway:8080`. Pointing `SITE_ADDRESS` at a real domain gets you
  Let's Encrypt automatically.

`Sharam.toml` is **not** read inside containers — figment falls through to
the `SHARAM_*` env vars injected by compose.

## Repository layout

```
crate/
  auth/       Google OIDC verifier, Argon2id passwords, session JWT
  common/     config, tracing, domain primitives, error type
  gateway/    axum HTTP entry point
  ledger/     SurrealDB client + schema (control plane + per-tenant)
  storage/    S3-compatible client scaffold (RustFS)
sharam-ui/    Dioxus frontend (web target)
docker/       Dockerfiles + Caddyfile
compose.yml   single-source-of-truth for dev and self-host
```

## Contributing

Issues and PRs welcome. Before submitting:

- `cargo fmt --all && cargo clippy --workspace --all-targets`
- `cargo test`
- Schemas must use `DEFINE … OVERWRITE` — re-applying must stay idempotent.
- Keep dependency versions on `{ workspace = true }`; pin in the root
  `Cargo.toml` only.

## License

MIT — see [LICENSE](./LICENSE).
