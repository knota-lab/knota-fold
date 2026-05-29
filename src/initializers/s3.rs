use std::sync::Arc;

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{config::Builder as S3ConfigBuilder, Client};
use loco_rs::{
    app::{AppContext, Initializer},
    Error, Result,
};
use serde::{Deserialize, Serialize};

/// S3/MinIO connection settings, sourced from `settings.s3` in loco config.
///
/// Field naming uses camelCase to match the project-wide front/back convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    /// Required for MinIO and most S3-compatible backends. Production AWS S3
    /// can set this to `false` for virtual-hosted style URLs.
    #[serde(default = "default_force_path_style")]
    pub force_path_style: bool,
}

fn default_force_path_style() -> bool {
    true
}

use crate::config::{AppSettings, ConfigExt};

pub type SharedS3Client = Arc<Client>;
pub type SharedS3Config = Arc<S3Config>;

pub struct S3ClientInitializer;

#[async_trait]
impl Initializer for S3ClientInitializer {
    fn name(&self) -> String {
        "s3".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        let settings: AppSettings = ctx
            .config
            .typed_settings()
            .map_err(|e| Error::Message(format!("invalid `settings` section: {e}")))?
            .ok_or_else(|| {
                Error::Message("`settings` section missing in config".to_string())
            })?;

        let cfg = settings.s3.ok_or_else(|| {
            Error::Message("`settings.s3` section missing in config".to_string())
        })?;

        let creds = Credentials::new(
            cfg.access_key.clone(),
            cfg.secret_key.clone(),
            None,
            None,
            "Static",
        );

        let shared_cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(cfg.region.clone()))
            .credentials_provider(creds)
            .endpoint_url(cfg.endpoint.clone())
            .load()
            .await;

        let s3_cfg = S3ConfigBuilder::from(&shared_cfg)
            .force_path_style(cfg.force_path_style)
            .build();

        let client = Client::from_conf(s3_cfg);

        ctx.shared_store.insert::<SharedS3Client>(Arc::new(client));
        ctx.shared_store
            .insert::<SharedS3Config>(Arc::new(cfg.clone()));

        tracing::info!(
            endpoint = %cfg.endpoint,
            region = %cfg.region,
            bucket = %cfg.bucket,
            force_path_style = cfg.force_path_style,
            "S3 client initialized and stored in shared_store"
        );

        Ok(())
    }
}
