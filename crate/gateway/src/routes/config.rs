use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
struct PublicConfig {
    google_client_id: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/config", get(public_config))
}

async fn public_config(State(state): State<AppState>) -> Json<PublicConfig> {
    Json(PublicConfig {
        google_client_id: state.google_client_id.clone(),
    })
}
