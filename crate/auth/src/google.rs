use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::debug;

use crate::error::{Error, Result};

/// Google's well-known issuer values. Tokens carry one of these.
const GOOGLE_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];
/// JWKS endpoint published in Google's OpenID discovery doc.
const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
/// How long to trust a fetched JWKS before refreshing. Google's
/// Cache-Control header typically allows a few hours; we play it safe.
const JWKS_TTL: Duration = Duration::from_secs(3600);

/// Verifies Google-issued ID tokens (RS256) and parses standard OIDC
/// claims. Cheap to clone — internals are `Arc`-ed.
#[derive(Clone)]
pub struct GoogleVerifier {
    inner: Arc<Inner>,
}

struct Inner {
    client_id: String,
    issuers: Vec<String>,
    jwks_url: String,
    http: reqwest::Client,
    cache: RwLock<Option<CachedJwks>>,
}

struct CachedJwks {
    keys: HashMap<String, Arc<DecodingKey>>,
    fetched_at: Instant,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleClaims {
    pub iss: String,
    pub aud: String,
    /// Stable, opaque Google user identifier — use as the primary join key.
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
    pub exp: i64,
    pub iat: i64,
}

impl GoogleVerifier {
    pub fn new(client_id: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client builds");
        Self {
            inner: Arc::new(Inner {
                client_id: client_id.into(),
                issuers: GOOGLE_ISSUERS.iter().map(|s| (*s).to_string()).collect(),
                jwks_url: GOOGLE_JWKS_URL.to_string(),
                http,
                cache: RwLock::new(None),
            }),
        }
    }

    /// Construct a verifier with a pre-populated key set. Used by tests so
    /// they can sign + verify locally without hitting Google.
    pub fn with_static_keys(
        client_id: impl Into<String>,
        issuers: Vec<String>,
        keys: HashMap<String, DecodingKey>,
    ) -> Self {
        let arc_keys = keys
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect::<HashMap<_, _>>();
        Self {
            inner: Arc::new(Inner {
                client_id: client_id.into(),
                issuers,
                jwks_url: String::new(),
                http: reqwest::Client::new(),
                cache: RwLock::new(Some(CachedJwks {
                    keys: arc_keys,
                    // Far in the past — but `with_static_keys` mode sets a
                    // sentinel by storing TTL'd-but-static keys; the
                    // refresh path is short-circuited because `jwks_url` is
                    // empty, so we keep the cache alive forever.
                    fetched_at: Instant::now(),
                })),
            }),
        }
    }

    /// Verify an ID token: signature, audience (`client_id`), issuer,
    /// expiry. Returns the parsed claims on success.
    pub async fn verify(&self, id_token: &str) -> Result<GoogleClaims> {
        let header =
            decode_header(id_token).map_err(|e| Error::InvalidToken(format!("header: {e}")))?;
        if header.alg != Algorithm::RS256 {
            return Err(Error::InvalidToken(format!(
                "unexpected alg: {:?}",
                header.alg
            )));
        }
        let kid = header
            .kid
            .ok_or_else(|| Error::InvalidToken("no kid in header".into()))?;

        let key = self.get_key(&kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.inner.client_id]);
        let issuers: Vec<&str> = self.inner.issuers.iter().map(|s| s.as_str()).collect();
        validation.set_issuer(&issuers);

        let data = decode::<GoogleClaims>(id_token, &key, &validation)
            .map_err(|e| Error::InvalidToken(e.to_string()))?;

        if !data.claims.email_verified {
            return Err(Error::InvalidToken("email not verified by Google".into()));
        }
        Ok(data.claims)
    }

    async fn get_key(&self, kid: &str) -> Result<Arc<DecodingKey>> {
        // Fast path — cache hit, still fresh.
        {
            let cache = self.inner.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && cached.fetched_at.elapsed() < JWKS_TTL
                && let Some(k) = cached.keys.get(kid)
            {
                return Ok(k.clone());
            }
        }
        // Refresh and look up again. If the URL is empty (test mode with
        // static keys), don't try to fetch — just fall through to lookup.
        if !self.inner.jwks_url.is_empty() {
            self.refresh_jwks().await?;
        }
        let cache = self.inner.cache.read().await;
        let cached = cache
            .as_ref()
            .ok_or_else(|| Error::OAuth("jwks unavailable".into()))?;
        cached
            .keys
            .get(kid)
            .cloned()
            .ok_or_else(|| Error::InvalidToken(format!("unknown kid: {kid}")))
    }

    async fn refresh_jwks(&self) -> Result<()> {
        debug!(url = %self.inner.jwks_url, "refreshing google jwks");
        let resp = self
            .inner
            .http
            .get(&self.inner.jwks_url)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::OAuth(format!("jwks fetch: {e}")))?;
        let jwks: GoogleJwks = resp
            .json()
            .await
            .map_err(|e| Error::OAuth(format!("jwks parse: {e}")))?;
        let mut keys = HashMap::with_capacity(jwks.keys.len());
        for jwk in jwks.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
                .map_err(|e| Error::OAuth(format!("invalid jwk: {e}")))?;
            keys.insert(jwk.kid, Arc::new(key));
        }
        let mut cache = self.inner.cache.write().await;
        *cache = Some(CachedJwks {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }
}

#[derive(Deserialize)]
struct GoogleJwks {
    keys: Vec<GoogleJwk>,
}

#[derive(Deserialize)]
struct GoogleJwk {
    kid: String,
    kty: String,
    n: String,
    e: String,
}
