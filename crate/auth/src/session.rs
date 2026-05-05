use std::sync::Arc;

use chrono::{Duration, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, errors::ErrorKind,
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Issuer claim baked into every Sharam-minted session JWT. Validated on
/// verify so a token signed by an unrelated service can't slip through.
pub const SESSION_ISSUER: &str = "sharam";

/// Default session lifetime when callers don't override. Short enough to
/// limit the blast radius of a stolen token, long enough that users don't
/// have to re-authenticate every browser session.
pub const DEFAULT_SESSION_TTL_DAYS: i64 = 7;

/// Identity carried by a verified session token. Used uniformly across
/// gateway routes regardless of whether the user signed in with Google or
/// email+password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub email: String,
    pub name: Option<String>,
    /// Avatar URL — only set for Google sign-ins. Carried purely for UI
    /// display (profile page); never used for authn.
    pub picture: Option<String>,
}

/// JWT claims for a Sharam session. `sub` is the user's email, which the
/// rest of the system already keys on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    pub iss: String,
    pub sub: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    pub exp: i64,
    pub iat: i64,
}

/// Mints and verifies session tokens with HS256. Cheap to clone — the
/// secret is `Arc`-ed.
#[derive(Clone)]
pub struct SessionSigner {
    enc: Arc<EncodingKey>,
    dec: Arc<DecodingKey>,
}

impl SessionSigner {
    /// Construct a signer from a shared HS256 secret. The secret should be
    /// at least 32 bytes of random data; rotation is handled by restarting
    /// the gateway with a new value (all live sessions invalidate).
    pub fn new(secret: impl AsRef<[u8]>) -> Self {
        let bytes = secret.as_ref();
        Self {
            enc: Arc::new(EncodingKey::from_secret(bytes)),
            dec: Arc::new(DecodingKey::from_secret(bytes)),
        }
    }

    /// Mint a session JWT with the default TTL.
    pub fn issue(&self, identity: &Identity) -> Result<String> {
        self.issue_with_ttl(identity, Duration::days(DEFAULT_SESSION_TTL_DAYS))
    }

    pub fn issue_with_ttl(&self, identity: &Identity, ttl: Duration) -> Result<String> {
        let now = Utc::now();
        let claims = SessionClaims {
            iss: SESSION_ISSUER.into(),
            sub: identity.email.clone(),
            email: identity.email.clone(),
            name: identity.name.clone(),
            picture: identity.picture.clone(),
            exp: (now + ttl).timestamp(),
            iat: now.timestamp(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.enc)
            .map_err(|e| Error::InvalidSession(format!("encode: {e}")))
    }

    /// Verify a session JWT: HS256 signature, our issuer, expiry. Returns
    /// the identity carried in the claims.
    pub fn verify(&self, token: &str) -> Result<Identity> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[SESSION_ISSUER]);
        validation.validate_exp = true;
        // jsonwebtoken's default 60s leeway exists to forgive client/server
        // clock drift on issuance — for sessions we want expired-is-expired.
        validation.leeway = 0;
        // Sharam-issued tokens have no `aud` claim — disable that check
        // so jsonwebtoken doesn't reject them with `MissingRequiredClaim`.
        validation.required_spec_claims =
            ["exp", "iss"].iter().map(|s| (*s).to_string()).collect();
        validation.set_audience::<&str>(&[]);
        validation.validate_aud = false;

        let data = decode::<SessionClaims>(token, &self.dec, &validation).map_err(|e| {
            match e.kind() {
                ErrorKind::ExpiredSignature => {
                    Error::InvalidSession("session expired".into())
                }
                _ => Error::InvalidSession(e.to_string()),
            }
        })?;
        Ok(Identity {
            email: data.claims.email,
            name: data.claims.name,
            picture: data.claims.picture,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = SessionSigner::new(b"this-is-a-test-secret-32-bytes!!");
        let id = Identity {
            email: "alice@example.com".into(),
            name: Some("Alice".into()),
            picture: None,
        };
        let tok = s.issue(&id).unwrap();
        let back = s.verify(&tok).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn rejects_wrong_secret() {
        let s1 = SessionSigner::new(b"secret-aaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let s2 = SessionSigner::new(b"secret-bbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let id = Identity {
            email: "alice@example.com".into(),
            name: None,
            picture: None,
        };
        let tok = s1.issue(&id).unwrap();
        assert!(matches!(s2.verify(&tok), Err(Error::InvalidSession(_))));
    }

    #[test]
    fn rejects_expired() {
        let s = SessionSigner::new(b"another-test-secret-32-bytes!!aa");
        let id = Identity {
            email: "alice@example.com".into(),
            name: None,
            picture: None,
        };
        let tok = s
            .issue_with_ttl(&id, Duration::seconds(-1))
            .unwrap();
        assert!(matches!(s.verify(&tok), Err(Error::InvalidSession(_))));
    }
}
