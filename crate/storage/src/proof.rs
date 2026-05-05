use std::time::Duration;

use aws_sdk_s3::{
    Client,
    config::{Credentials, Region},
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{BucketLocationConstraint, CreateBucketConfiguration},
};
use common::config::StorageConfig;
use tracing::info;
use uuid::Uuid;

use crate::error::{Error, Result};

/// Common key prefix for every stored proof. All keys live under
/// `tenants/{slug}/...` so the gateway can authorize reads by tenant
/// without consulting the database.
pub const PROOF_KEY_PREFIX: &str = "tenants";

/// S3-compatible store for contribution proof-of-payment files. Backs onto
/// either RustFS (the default in `compose.yml`) or AWS S3 — same client,
/// just point `[storage].endpoint` somewhere else.
///
/// Keys are server-generated as `tenants/{slug}/{email-frag}/{uuidv7}.{ext}`.
/// Callers cannot pick keys, so a member can't claim someone else's upload.
#[derive(Clone)]
pub struct ProofStore {
    client: Client,
    bucket: String,
    region: String,
}

impl ProofStore {
    /// Build an S3 client from `[storage]` config. `force_path_style(true)`
    /// is required for RustFS / MinIO; AWS also accepts it.
    pub fn from_config(cfg: &StorageConfig) -> Result<Self> {
        if cfg.bucket.is_empty() {
            return Err(Error::S3("storage.bucket is empty".into()));
        }
        let creds = Credentials::new(
            &cfg.access_key_id,
            &cfg.secret_access_key,
            None,
            None,
            "sharam-static",
        );
        let region = Region::new(cfg.region.clone());
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(region)
            .credentials_provider(creds)
            .force_path_style(true);
        // An empty endpoint means "use the AWS default for this region".
        if !cfg.endpoint.is_empty() {
            builder = builder.endpoint_url(&cfg.endpoint);
        }
        Ok(Self {
            client: Client::from_conf(builder.build()),
            bucket: cfg.bucket.clone(),
            region: cfg.region.clone(),
        })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Make sure the configured bucket exists. Idempotent — swallows the
    /// "already owned by you / already exists" race so repeated boots are
    /// safe. Run once at gateway startup.
    pub async fn ensure_bucket(&self) -> Result<()> {
        let mut req = self.client.create_bucket().bucket(&self.bucket);
        // S3 rejects a LocationConstraint of `us-east-1` (it's the implicit
        // default); other regions require it. RustFS accepts either.
        if self.region != "us-east-1" {
            let constraint = BucketLocationConstraint::from(self.region.as_str());
            let bucket_cfg = CreateBucketConfiguration::builder()
                .location_constraint(constraint)
                .build();
            req = req.create_bucket_configuration(bucket_cfg);
        }
        match req.send().await {
            Ok(_) => {
                info!(bucket = %self.bucket, "created storage bucket");
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("BucketAlreadyOwnedByYou") || msg.contains("BucketAlreadyExists") {
                    Ok(())
                } else {
                    Err(Error::S3(msg))
                }
            }
        }
    }

    /// Store an uploaded proof. Returns the server-chosen key.
    pub async fn put(
        &self,
        slug: &str,
        user_email: &str,
        content_type: &str,
        ext: &str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        let id = Uuid::now_v7();
        let frag = sanitize_email_frag(user_email);
        let key = format!("{PROOF_KEY_PREFIX}/{slug}/{frag}/{id}.{ext}");
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .content_type(content_type)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|e| Error::S3(e.to_string()))?;
        Ok(key)
    }

    /// Time-limited GET URL the browser can fetch directly. Use a short TTL
    /// (a few minutes) — the URL is bearer-equivalent for the duration.
    pub async fn presign_get(&self, key: &str, ttl: Duration) -> Result<String> {
        let presigning =
            PresigningConfig::expires_in(ttl).map_err(|e| Error::S3(e.to_string()))?;
        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| Error::S3(e.to_string()))?;
        Ok(req.uri().to_string())
    }
}

/// Filesystem-safe path fragment for an email. `alice@example.com` →
/// `alice_at_example.com`. Lowercased; non-`[a-z0-9._-]` runs collapse to
/// `-`. Not reversible — the contribution row's `user_email` is the source
/// of truth for "who uploaded this".
fn sanitize_email_frag(email: &str) -> String {
    let lower = email.to_lowercase();
    let merged = match lower.split_once('@') {
        Some((local, domain)) => format!("{local}_at_{domain}"),
        None => lower,
    };
    let mut out = String::with_capacity(merged.len());
    let mut prev_dash = false;
    for c in merged.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_typical_email() {
        assert_eq!(
            sanitize_email_frag("Alice.Smith+ledger@Example.com"),
            "alice.smith-ledger_at_example.com"
        );
    }

    #[test]
    fn sanitize_no_at() {
        assert_eq!(sanitize_email_frag("nobody"), "nobody");
    }
}
