//! Offline tests for `GoogleVerifier`. We mint a self-signed RS256 token
//! against an in-memory RSA keypair, register the matching JWK in the
//! verifier via `with_static_keys`, and verify happy + unhappy paths.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use auth::{Error, GoogleVerifier};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
use rand_core::OsRng;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::EncodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::Serialize;

const TEST_CLIENT_ID: &str = "test-client.apps.googleusercontent.com";
const TEST_KID: &str = "test-kid-1";
const TEST_ISSUER: &str = "https://accounts.google.com";

#[derive(Serialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    email: String,
    email_verified: bool,
    name: Option<String>,
    exp: i64,
    iat: i64,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct TestEnv {
    verifier: GoogleVerifier,
    enc_key: EncodingKey,
}

fn build_env(client_id: &str, issuer: &str) -> TestEnv {
    let mut rng = OsRng;
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
    let pub_key = RsaPublicKey::from(&priv_key);

    // jsonwebtoken needs PEM input.
    let priv_pem = priv_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("priv pem");
    let pub_pem = pub_key
        .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
        .expect("pub pem");

    let enc_key = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("enc key");
    let dec_key = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec key");

    // Sanity-check: `n` and `e` of the public key are what a real JWK would
    // ship. We don't actually need them since `with_static_keys` consumes a
    // pre-built DecodingKey, but assert encoding works for completeness.
    let _n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let _e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());

    let mut keys = HashMap::new();
    keys.insert(TEST_KID.to_string(), dec_key);

    let verifier =
        GoogleVerifier::with_static_keys(client_id.to_string(), vec![issuer.to_string()], keys);

    TestEnv { verifier, enc_key }
}

fn make_token(env: &TestEnv, claims: Claims) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string());
    encode(&header, &claims, &env.enc_key).expect("sign")
}

#[tokio::test]
async fn happy_path_verified_email() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    let now = now();
    let token = make_token(
        &env,
        Claims {
            iss: TEST_ISSUER.into(),
            aud: TEST_CLIENT_ID.into(),
            sub: "google-sub-123".into(),
            email: "user@example.com".into(),
            email_verified: true,
            name: Some("Test User".into()),
            iat: now - 5,
            exp: now + 600,
        },
    );

    let claims = env.verifier.verify(&token).await.expect("verify");
    assert_eq!(claims.sub, "google-sub-123");
    assert_eq!(claims.email, "user@example.com");
    assert!(claims.email_verified);
}

#[tokio::test]
async fn rejects_unverified_email() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    let now = now();
    let token = make_token(
        &env,
        Claims {
            iss: TEST_ISSUER.into(),
            aud: TEST_CLIENT_ID.into(),
            sub: "google-sub-123".into(),
            email: "user@example.com".into(),
            email_verified: false,
            name: None,
            iat: now - 5,
            exp: now + 600,
        },
    );
    let err = env.verifier.verify(&token).await.expect_err("must reject");
    matches!(err, Error::InvalidToken(_));
}

#[tokio::test]
async fn rejects_wrong_audience() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    let now = now();
    let token = make_token(
        &env,
        Claims {
            iss: TEST_ISSUER.into(),
            aud: "someone-else.apps.googleusercontent.com".into(),
            sub: "x".into(),
            email: "x@y.z".into(),
            email_verified: true,
            name: None,
            iat: now - 5,
            exp: now + 600,
        },
    );
    let err = env.verifier.verify(&token).await.expect_err("must reject");
    matches!(err, Error::InvalidToken(_));
}

#[tokio::test]
async fn rejects_wrong_issuer() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    let now = now();
    let token = make_token(
        &env,
        Claims {
            iss: "https://evil.example.com".into(),
            aud: TEST_CLIENT_ID.into(),
            sub: "x".into(),
            email: "x@y.z".into(),
            email_verified: true,
            name: None,
            iat: now - 5,
            exp: now + 600,
        },
    );
    let err = env.verifier.verify(&token).await.expect_err("must reject");
    matches!(err, Error::InvalidToken(_));
}

#[tokio::test]
async fn rejects_expired() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    let now = now();
    let token = make_token(
        &env,
        Claims {
            iss: TEST_ISSUER.into(),
            aud: TEST_CLIENT_ID.into(),
            sub: "x".into(),
            email: "x@y.z".into(),
            email_verified: true,
            name: None,
            iat: now - 7200,
            exp: now - 3600,
        },
    );
    let err = env.verifier.verify(&token).await.expect_err("must reject");
    matches!(err, Error::InvalidToken(_));
}

#[tokio::test]
async fn rejects_unknown_kid() {
    let env = build_env(TEST_CLIENT_ID, TEST_ISSUER);
    // Sign with the right key but advertise a different kid in the header.
    let now = now();
    let claims = Claims {
        iss: TEST_ISSUER.into(),
        aud: TEST_CLIENT_ID.into(),
        sub: "x".into(),
        email: "x@y.z".into(),
        email_verified: true,
        name: None,
        iat: now - 5,
        exp: now + 600,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("not-a-known-kid".into());
    let token = encode(&header, &claims, &env.enc_key).unwrap();

    let err = env.verifier.verify(&token).await.expect_err("must reject");
    matches!(err, Error::InvalidToken(_));
}
