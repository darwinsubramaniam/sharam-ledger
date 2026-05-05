use std::sync::Arc;

use auth::{GoogleVerifier, SessionSigner};
use ledger::Ledger;

use crate::mailer::Mailer;

#[derive(Clone)]
pub struct AppState {
    /// Verifies Google ID tokens posted to `/api/auth/google`. Not used by
    /// any other route — protected routes verify session JWTs locally.
    pub google: Arc<GoogleVerifier>,
    /// Issues + verifies Sharam session JWTs (HS256). Both `/api/auth/google`
    /// and `/api/auth/login` mint one of these on success; every protected
    /// route verifies one to authenticate the caller.
    pub sessions: SessionSigner,
    pub ledger: Ledger,
    pub mailer: Mailer,
}
