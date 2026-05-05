mod mailer;
mod routes;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use auth::{GoogleVerifier, SessionSigner};
use axum::Router;
use http::{HeaderValue, Method, header};
use ledger::Ledger;
use storage::ProofStore;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use common::config::AppConfig;
use mailer::Mailer;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::tracing_init::init("gateway");

    let cfg = AppConfig::load().context("loading config")?;

    let ledger = Ledger::connect(cfg.surrealdb.clone())
        .await
        .context("connecting ledger")?;
    ledger
        .apply_control_schema()
        .await
        .context("applying control schema")?;
    ledger
        .migrate_all_tenants()
        .await
        .context("migrating tenant schemas")?;

    let proofs = ProofStore::from_config(&cfg.storage).context("building proof store")?;
    proofs
        .ensure_bucket()
        .await
        .context("ensuring proof bucket exists")?;

    let app = build_app(&cfg, ledger, proofs)?;

    let addr: SocketAddr = cfg
        .gateway
        .bind
        .parse()
        .with_context(|| format!("invalid bind address: {}", cfg.gateway.bind))?;

    info!(%addr, "gateway listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn build_app(cfg: &AppConfig, ledger: Ledger, proofs: ProofStore) -> anyhow::Result<Router> {
    let cors = CorsLayer::new()
        .allow_origin(
            cfg.gateway
                .frontend_origin
                .parse::<HeaderValue>()
                .context("invalid frontend_origin")?,
        )
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_credentials(true);

    let mailer = Mailer::new(&cfg.smtp).context("building smtp mailer")?;

    let secret = cfg.gateway.session_secret.as_bytes();
    if secret.len() < 32 {
        return Err(anyhow!(
            "gateway.session_secret must be at least 32 bytes (got {})",
            secret.len()
        ));
    }

    let state = AppState {
        google: Arc::new(GoogleVerifier::new(&cfg.google.client_id)),
        sessions: SessionSigner::new(secret),
        ledger,
        mailer,
        proofs,
    };

    let app = Router::new()
        .merge(routes::health::router())
        .merge(routes::auth::router())
        .merge(routes::tenants::router())
        .merge(routes::me::router())
        .merge(routes::settings::router())
        .merge(routes::invites::router())
        .merge(routes::members::router())
        .merge(routes::contributions::router())
        .merge(routes::carry_forward::router())
        .merge(routes::proofs::router())
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    Ok(app)
}
