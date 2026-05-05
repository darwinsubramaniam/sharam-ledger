#!/usr/bin/env bash
# dev.sh — bring up Sharam's local backing services (SurrealDB + RustFS +
# mailhog) and print the connection values you need to drop into
# `Sharam.toml` (host-mode dev, the default) or `.env` (full-stack
# `--profile app`). Run from the repo root.
#
#   ./dev.sh               same as `up`
#   ./dev.sh up                pull, start, wait for /health, print config
#   ./dev.sh print             just print the config (services already up)
#   ./dev.sh down              stop services; data preserved in named volumes
#   ./dev.sh nuke              stop + delete ALL volumes (irreversible)
#   ./dev.sh reset surrealdb   wipe just the surrealdb volume (irreversible)
#   ./dev.sh reset rustfs      wipe just the rustfs volume (irreversible)
#   ./dev.sh status            docker compose ps
#   ./dev.sh logs              tail logs from all running services

set -euo pipefail

cd "$(dirname "$0")"

# ── Resolve docker compose ───────────────────────────────────────────────
# Require v2 (`docker compose`, with a space). v1's `docker-compose`
# binary doesn't support `--wait` and is EOL'd anyway.
if ! command -v docker >/dev/null 2>&1; then
    echo "error: docker not found. Install Docker Desktop or the docker engine." >&2
    exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
    echo "error: 'docker compose' (v2) not available. Install the compose plugin." >&2
    exit 1
fi
DC=(docker compose)

# ── Source .env (or .env.example as fallback) ────────────────────────────
# We read these so the printed values reflect what compose actually
# interpolated, not just the example defaults.
if [[ -f .env ]]; then
    ENV_SOURCE=".env"
elif [[ -f .env.example ]]; then
    ENV_SOURCE=".env.example"
else
    ENV_SOURCE=""
fi
if [[ -n "$ENV_SOURCE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_SOURCE"
    set +a
fi

SURREAL_PASS="${SURREAL_PASS:-root}"
RUSTFS_USER="${RUSTFS_USER:-rustfs}"
RUSTFS_PASS="${RUSTFS_PASS:-rustfs}"
STORAGE_BUCKET="${STORAGE_BUCKET:-sharam-proofs}"
STORAGE_REGION="${STORAGE_REGION:-us-east-1}"

# ── Colors when stdout is a tty ──────────────────────────────────────────
if [[ -t 1 ]]; then
    BOLD=$'\033[1m'; DIM=$'\033[2m'
    CYAN=$'\033[36m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'
    RESET=$'\033[0m'
else
    BOLD=""; DIM=""; CYAN=""; GREEN=""; YELLOW=""; RESET=""
fi

print_config() {
    cat <<EOF

${BOLD}── Sharam dev services ─────────────────────────────────────────${RESET}
  ${CYAN}SurrealDB${RESET}       ws://127.0.0.1:8000
  ${CYAN}RustFS S3${RESET}       http://127.0.0.1:9000
  ${CYAN}RustFS console${RESET}  http://127.0.0.1:9001  ${DIM}(sign in: ${RUSTFS_USER} / ${RUSTFS_PASS})${RESET}
  ${CYAN}Mailhog SMTP${RESET}    127.0.0.1:1025         ${DIM}(no auth)${RESET}
  ${CYAN}Mailhog UI${RESET}      http://127.0.0.1:8025

${BOLD}Sharam.toml${RESET} ${DIM}— host-mode dev (\`cargo run -p gateway\`)${RESET}

  [surrealdb]
  endpoint = "ws://127.0.0.1:8000"
  namespace = "sharam"
  database = "main"
  username = "root"
  password = "${SURREAL_PASS}"

  [storage]
  endpoint = "http://127.0.0.1:9000"
  region = "${STORAGE_REGION}"
  bucket = "${STORAGE_BUCKET}"
  access_key_id = "${RUSTFS_USER}"
  secret_access_key = "${RUSTFS_PASS}"

  [smtp]
  host = "127.0.0.1"
  port = 1025
  encryption = "plain"
  username = ""
  password = ""
  from_email = "dev@sharam.local"
  from_name = "Sharam (dev)"
  app_base_url = "http://localhost:3000"

${BOLD}.env${RESET} ${DIM}— full stack (\`docker compose --profile app up -d --build\`)${RESET}
  Copy ${CYAN}.env.example${RESET} → ${CYAN}.env${RESET} and fill in the REPLACE_* placeholders.
  The same RUSTFS_USER / RUSTFS_PASS feed both the rustfs container and
  the gateway's SHARAM_STORAGE__* — they cannot drift.

${BOLD}Next${RESET}
  ${DIM}# in two terminals:${RESET}
  cargo run -p gateway
  dx serve --package sharam-ui

EOF
}

cmd="${1:-up}"

case "$cmd" in
    up)
        if [[ ! -f Sharam.toml && -f Sharam.example.toml ]]; then
            echo "${YELLOW}note:${RESET} Sharam.toml is missing — copy from the example before \`cargo run -p gateway\`:"
            echo "  cp Sharam.example.toml Sharam.toml"
            echo
        fi
        echo "→ Pulling images (cache hit = no-op)..."
        "${DC[@]}" pull --quiet surrealdb rustfs mailhog || true
        echo "→ Starting backing services..."
        # --wait blocks until each service reports healthy (or `started`
        # for those without a healthcheck — mailhog). Default timeout is
        # ~60s; bump RUSTFS healthcheck.start_period in compose.yml if
        # you see flakes here on slow disks.
        "${DC[@]}" up -d --wait surrealdb rustfs mailhog
        echo "${GREEN}✓${RESET} backing services healthy"
        print_config
        ;;
    print)
        print_config
        ;;
    down)
        "${DC[@]}" down
        ;;
    nuke)
        echo "${YELLOW}!${RESET} this DELETES all SurrealDB + RustFS + mailhog data."
        echo "  type 'nuke' to confirm:"
        read -r answer
        if [[ "$answer" == "nuke" ]]; then
            "${DC[@]}" down -v
            echo "${GREEN}✓${RESET} stopped and volumes deleted"
        else
            echo "aborted"
            exit 1
        fi
        ;;
    reset)
        # Wipe a single service's data volume — useful when an image bump
        # changes the runtime UID and the existing volume's ownership no
        # longer matches (e.g. surrealdb v3.0.5 runs as uid 65532, an
        # older `latest` ran as root → "Permission denied" on /data).
        target="${2:-}"
        case "$target" in
            surrealdb) volume_name="sharam_surreal-data" ;;
            rustfs)    volume_name="sharam_rustfs-data"  ;;
            *)
                echo "usage: $0 reset {surrealdb|rustfs}" >&2
                exit 2
                ;;
        esac
        echo "${YELLOW}!${RESET} this DELETES the ${target} data volume (${volume_name})."
        echo "  type 'reset' to confirm:"
        read -r answer
        if [[ "$answer" != "reset" ]]; then
            echo "aborted"
            exit 1
        fi
        "${DC[@]}" rm -fsv "$target" >/dev/null 2>&1 || true
        docker volume rm "$volume_name" >/dev/null 2>&1 || true
        echo "${GREEN}✓${RESET} ${target} volume deleted — run \`./dev.sh up\` to recreate"
        ;;
    status)
        "${DC[@]}" ps
        ;;
    logs)
        "${DC[@]}" logs -f --tail=200
        ;;
    -h|--help|help)
        sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
        ;;
    *)
        echo "error: unknown command '${cmd}'" >&2
        echo "usage: $0 [up|print|down|nuke|reset {surrealdb|rustfs}|status|logs]" >&2
        exit 2
        ;;
esac
