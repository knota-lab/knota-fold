use async_trait::async_trait;
use base64::Engine as _;
use loco_rs::{
    app::{AppContext, Initializer},
    Error, Result,
};

use crate::config::ConfigExt;

pub struct ConfigValidator;

#[async_trait]
impl Initializer for ConfigValidator {
    fn name(&self) -> String {
        "config_validator".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        if let Some(settings) = ctx
            .config
            .typed_settings()
            .map_err(|e| Error::Message(format!("invalid `settings` section: {e}")))?
        {
            if let Err(errors) = settings.validate() {
                for error in &errors {
                    tracing::error!(error = %error, "CONFIG VALIDATION ERROR");
                }
                return Err(Error::Message(format!(
                    "config validation failed with {} error(s); see logs above",
                    errors.len()
                )));
            }
        }

        let jwt_cfg = ctx
            .config
            .get_jwt_config()
            .map_err(|e| Error::Message(format!("JWT config missing: {e}")))?;

        if base64::engine::general_purpose::STANDARD
            .decode(&jwt_cfg.secret)
            .is_err()
        {
            let secret_preview_len = jwt_cfg.secret.len().min(8);
            tracing::error!(
                secret_preview = %&jwt_cfg.secret[..secret_preview_len],
                "JWT secret is not valid Base64. loco-rs uses EncodingKey::from_base64_secret(). Generate with: openssl rand -base64 32"
            );
            return Err(Error::Message(
                "auth.jwt.secret is not valid Base64 — JWT generation will silently fail"
                    .to_string(),
            ));
        }

        let dev_secrets = [
            "change-me-dev-jwt-secret-base64",
            "test-jwt-secret-base64-change-me",
        ];
        if dev_secrets.contains(&jwt_cfg.secret.as_str()) {
            tracing::warn!(
                "⚠️  JWT secret appears to be a development/default value — change it in production!"
            );
        }

        tracing::info!("Configuration validation passed");
        Ok(())
    }
}
