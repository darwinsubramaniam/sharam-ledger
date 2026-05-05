use std::sync::Arc;

use auth::GoogleVerifier;
use ledger::Ledger;

use crate::mailer::Mailer;

#[derive(Clone)]
pub struct AppState {
    pub google: Arc<GoogleVerifier>,
    pub ledger: Ledger,
    pub mailer: Mailer,
}
