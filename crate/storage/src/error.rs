use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("s3: {0}")]
    S3(String),

    #[error("not found")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, Error>;
