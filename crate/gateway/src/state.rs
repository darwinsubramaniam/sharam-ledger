use std::sync::Arc;

use auth::{GoogleVerifier, SessionSigner};
use ledger::Ledger;
use storage::ProofStore;

use crate::mailer::Mailer;

#[derive(Clone)]
pub struct AppState {
    /// Verifies Google ID tokens posted to `/api/auth/google`. Not used by
    /// any other route — protected routes verify session JWTs locally.
    pub google: Arc<GoogleVerifier>,
    /// OAuth 2.0 client ID for Google sign-in. Served to the frontend via
    /// `GET /api/config` so the wasm bundle stays deployment-agnostic.
    pub google_client_id: String,
    /// Issues + verifies Sharam session JWTs (HS256). Both `/api/auth/google`
    /// and `/api/auth/login` mint one of these on success; every protected
    /// route verifies one to authenticate the caller.
    pub sessions: SessionSigner,
    pub ledger: Ledger,
    pub mailer: Mailer,
    /// S3-compatible store for contribution proof-of-payment files
    /// (`POST /api/tenants/:slug/proofs` writes here).
    pub proofs: ProofStore,
}
