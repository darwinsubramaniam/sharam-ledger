mod mailer;
mod routes;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use auth::GoogleVerifier;
use axum::Router;
use http::{HeaderValue, Method, header};
use ledger::Ledger;
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

    let app = build_app(&cfg, ledger)?;

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

fn build_app(cfg: &AppConfig, ledger: Ledger) -> anyhow::Result<Router> {
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

    let state = AppState {
        google: Arc::new(GoogleVerifier::new(&cfg.google.client_id)),
        ledger,
        mailer,
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
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    Ok(app)
}
