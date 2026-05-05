use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("invalid period: {0}")]
    InvalidPeriod(String),

    #[error("invalid cadence: {0}")]
    InvalidCadence(String),

    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),

    #[error("invalid tenant slug: {0}")]
    InvalidTenantSlug(String),
}

pub type Result<T> = std::result::Result<T, Error>;
