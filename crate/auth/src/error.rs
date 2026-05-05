use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid id token: {0}")]
    InvalidToken(String),

    #[error("not invited")]
    NotInvited,

    #[error("oauth: {0}")]
    OAuth(String),

    #[error("network: {0}")]
    Network(#[from] reqwest::Error),

    #[error("invalid session token: {0}")]
    InvalidSession(String),

    #[error("password hash: {0}")]
    PasswordHash(String),

    #[error("invalid password")]
    PasswordMismatch,
}

pub type Result<T> = std::result::Result<T, Error>;
