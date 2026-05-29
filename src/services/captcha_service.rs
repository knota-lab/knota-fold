//! Stateless image CAPTCHA service for `/api/auth/login`.
//!
//! Uses [`captcha-rs`] with the `stateless` feature: the answer is signed into
//! a JWT (`token`) that is returned to the client alongside the base64 image.
//! The client echoes the token + the user-typed solution back on login;
//! verification is purely cryptographic — no server-side per-captcha state.
//!
//! The JWT signing secret + image params come from `settings.captcha` in the
//! loco config (see `config/development.yaml`). Rotating `secretKey`
//! invalidates all outstanding captcha tokens.

use std::sync::OnceLock;

use captcha_rs::CaptchaBuilder;
use loco_rs::{app::AppContext, Error, Result};
use serde::Deserialize;

use crate::config::ConfigExt;

/// Parsed `settings.captcha` config block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptchaConfig {
    /// JWT signing secret. Must be non-empty.
    pub secret_key: String,
    /// Token validity window in seconds (also the captcha's effective lifetime).
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,
    #[serde(default = "default_length")]
    pub length: usize,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    /// 1..=10. Higher = noisier image.
    #[serde(default = "default_complexity")]
    pub complexity: u32,
    #[serde(default)]
    pub dark_mode: bool,
}

fn default_ttl() -> u64 {
    300
}
fn default_length() -> usize {
    5
}
fn default_width() -> u32 {
    130
}
fn default_height() -> u32 {
    40
}
fn default_complexity() -> u32 {
    3
}

/// Process-wide cache of the parsed config so we don't re-parse `settings`
/// on every request. Config is read once at first use; restarting the process
/// is required to pick up changes (same lifecycle as JWT secret).
static CONFIG_CACHE: OnceLock<CaptchaConfig> = OnceLock::new();

fn load_config(ctx: &AppContext) -> Result<&'static CaptchaConfig> {
    if let Some(cfg) = CONFIG_CACHE.get() {
        return Ok(cfg);
    }

    let settings = ctx
        .config
        .typed_settings()
        .map_err(|e| Error::Message(format!("invalid `settings` section: {e}")))?
        .ok_or_else(|| {
            Error::Message("`settings` section missing in config".to_string())
        })?;
    let cfg = settings.captcha.ok_or_else(|| {
        Error::Message("`settings.captcha` section missing in config".to_string())
    })?;
    if cfg.secret_key.trim().is_empty() {
        return Err(Error::Message(
            "`settings.captcha.secretKey` must be non-empty".to_string(),
        ));
    }

    // First writer wins; if a concurrent caller raced us, drop ours and use theirs.
    let _ = CONFIG_CACHE.set(cfg);
    Ok(CONFIG_CACHE.get().expect("CONFIG_CACHE just set"))
}

/// Generate a fresh captcha. Returns `(image_data_url, signed_token)` where
/// `image_data_url` is a `data:image/jpeg;base64,...` string ready for an
/// `<img src>` and `signed_token` is the opaque JWT to be echoed back on
/// login.
pub fn generate(ctx: &AppContext) -> Result<(String, String)> {
    let cfg = load_config(ctx)?;

    let captcha = CaptchaBuilder::new()
        .length(cfg.length)
        .width(cfg.width)
        .height(cfg.height)
        .dark_mode(cfg.dark_mode)
        .complexity(cfg.complexity as u32)
        .build();

    captcha
        .as_tuple(&cfg.secret_key, cfg.ttl_seconds)
        .ok_or_else(|| Error::Message("failed to sign captcha token".to_string()))
}

/// Verify a (token, user_solution) pair. Returns `Ok(true)` on success;
/// `Ok(false)` for any verification failure (wrong answer / expired /
/// tampered token / missing input) — the caller surfaces a single
/// "captcha invalid" error to the client without leaking which case it was.
pub fn verify(ctx: &AppContext, token: &str, solution: &str) -> Result<bool> {
    if token.is_empty() || solution.is_empty() {
        return Ok(false);
    }
    let cfg = load_config(ctx)?;
    // captcha_rs::verify returns:
    //   None         => token invalid / expired / wrong secret
    //   Some(false)  => token ok but answer mismatch
    //   Some(true)   => pass
    Ok(captcha_rs::verify(token, solution, &cfg.secret_key).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure crypto round-trip — does not need an AppContext. Validates that
    /// the captcha-rs stateless feature is actually wired up (i.e. the
    /// crate-level `verify` symbol exists with the expected signature).
    #[test]
    fn captcha_rs_round_trip() {
        let secret = "unit-test-secret";
        let captcha = CaptchaBuilder::new()
            .text("abcde".to_string())
            .length(5)
            .build();
        let (img, token) = captcha
            .as_tuple(secret, 60)
            .expect("token signing must succeed");
        assert!(img.starts_with("data:image/jpeg;base64,"));

        // captcha-rs lower-cases internally
        assert_eq!(captcha_rs::verify(&token, "abcde", secret), Some(true));
        assert_eq!(captcha_rs::verify(&token, "ABCDE", secret), Some(true));
        assert_eq!(captcha_rs::verify(&token, "wrong", secret), Some(false));
        assert!(captcha_rs::verify(&token, "abcde", "other-secret").is_none());
        assert!(captcha_rs::verify("bogus.token.value", "abcde", secret).is_none());
    }
}
