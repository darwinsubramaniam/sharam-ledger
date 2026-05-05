use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::error::{Error, Result};

/// Hash `password` with Argon2id using a fresh random salt. Returns a
/// PHC-format string (`$argon2id$v=19$m=...$...`) safe to store in the DB.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::PasswordHash(e.to_string()))
}

/// Verify `password` against the stored PHC `hash`. Returns `Ok(())` on
/// match, `Err(PasswordMismatch)` on mismatch, `Err(PasswordHash(_))` on
/// malformed hash.
pub fn verify_password(password: &str, hash: &str) -> Result<()> {
    let parsed = PasswordHash::new(hash).map_err(|e| Error::PasswordHash(e.to_string()))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| Error::PasswordMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = hash_password("hunter2").unwrap();
        assert!(h.starts_with("$argon2"));
        verify_password("hunter2", &h).unwrap();
        assert!(matches!(
            verify_password("wrong", &h),
            Err(Error::PasswordMismatch)
        ));
    }
}
