//! Login throttling & lock-out for `/api/auth/login`.
//!
//! Tracks per-account login failures in the loco cache (InMem in dev →
//! Redis in prod, transparent to this module). Two cache keys per account:
//!
//!   * `auth:login:fail:{email}` → `i64` running failure counter (TTL =
//!     `failure_window_seconds`)
//!   * `auth:login:lock:{email}` → `i64` lock-until epoch seconds
//!     (TTL = `lock_duration_seconds`)
//!
//! Thresholds & TTLs are sourced from `sys_configs` (global scope) so the
//! security team can tune them at runtime without a redeploy. Defaults are
//! conservative production values:
//!
//!   * `auth.login.max_failures_before_captcha`  = 3
//!   * `auth.login.max_failures_before_lock`     = 10
//!   * `auth.login.lock_duration_seconds`        = 900   (15 min)
//!   * `auth.login.failure_window_seconds`       = 900   (15 min)
//!   * `auth.login.captcha_required`             = false (force on every login?)
//!
//! Email is normalised to lowercase to match `users::find_by_email` and to
//! prevent trivial bypass via case variation.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use loco_rs::{app::AppContext, cache};

use crate::services::sys_config_service;

// ── sys_configs keys ────────────────────────────────────────────────────

const KEY_CAPTCHA_REQUIRED: &str = "auth.login.captcha_required";
const KEY_MAX_FAIL_CAPTCHA: &str = "auth.login.max_failures_before_captcha";
const KEY_MAX_FAIL_LOCK: &str = "auth.login.max_failures_before_lock";
const KEY_LOCK_DURATION: &str = "auth.login.lock_duration_seconds";
const KEY_FAILURE_WINDOW: &str = "auth.login.failure_window_seconds";

// ── Defaults (mirror fixtures/sys_configs.yaml) ─────────────────────────

const DEFAULT_CAPTCHA_REQUIRED: bool = false;
const DEFAULT_MAX_FAIL_CAPTCHA: i64 = 3;
const DEFAULT_MAX_FAIL_LOCK: i64 = 10;
const DEFAULT_LOCK_DURATION: i64 = 900;
const DEFAULT_FAILURE_WINDOW: i64 = 900;

// ── Cache key helpers ───────────────────────────────────────────────────

fn fail_key(email: &str) -> String {
    format!("auth:login:fail:{}", email.to_lowercase())
}
fn lock_key(email: &str) -> String {
    format!("auth:login:lock:{}", email.to_lowercase())
}

// ── Threshold loading ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LoginThresholds {
    pub captcha_required: bool,
    pub max_failures_before_captcha: i64,
    pub max_failures_before_lock: i64,
    pub lock_duration_seconds: i64,
    pub failure_window_seconds: i64,
}

impl Default for LoginThresholds {
    fn default() -> Self {
        Self {
            captcha_required: DEFAULT_CAPTCHA_REQUIRED,
            max_failures_before_captcha: DEFAULT_MAX_FAIL_CAPTCHA,
            max_failures_before_lock: DEFAULT_MAX_FAIL_LOCK,
            lock_duration_seconds: DEFAULT_LOCK_DURATION,
            failure_window_seconds: DEFAULT_FAILURE_WINDOW,
        }
    }
}

async fn load_int(ctx: &AppContext, key: &str, default: i64) -> i64 {
    match sys_config_service::get_resolved_detail(ctx, key, None).await {
        Ok(Some(detail)) => detail.resolved_value.parse::<i64>().unwrap_or(default),
        _ => default,
    }
}

async fn load_bool(ctx: &AppContext, key: &str, default: bool) -> bool {
    match sys_config_service::get_resolved_detail(ctx, key, None).await {
        Ok(Some(detail)) => match detail.resolved_value.as_str() {
            "true" => true,
            "false" => false,
            _ => default,
        },
        _ => default,
    }
}

/// Read all five thresholds from `sys_configs` (global). Falls back to
/// hard-coded defaults if the config key is missing or malformed; never
/// errors so a misconfigured key cannot lock everyone out.
pub async fn load_thresholds(ctx: &AppContext) -> LoginThresholds {
    LoginThresholds {
        captcha_required: load_bool(ctx, KEY_CAPTCHA_REQUIRED, DEFAULT_CAPTCHA_REQUIRED)
            .await,
        max_failures_before_captcha: load_int(
            ctx,
            KEY_MAX_FAIL_CAPTCHA,
            DEFAULT_MAX_FAIL_CAPTCHA,
        )
        .await,
        max_failures_before_lock: load_int(ctx, KEY_MAX_FAIL_LOCK, DEFAULT_MAX_FAIL_LOCK)
            .await,
        lock_duration_seconds: load_int(ctx, KEY_LOCK_DURATION, DEFAULT_LOCK_DURATION)
            .await,
        failure_window_seconds: load_int(ctx, KEY_FAILURE_WINDOW, DEFAULT_FAILURE_WINDOW)
            .await,
    }
}

// ── Counter & lock primitives ───────────────────────────────────────────

async fn get_fail_count(cache: &Arc<cache::Cache>, email: &str) -> i64 {
    cache
        .get::<i64>(&fail_key(email))
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// Returns the lock-until epoch (seconds) if the account is currently locked.
pub async fn get_lock_until(cache: &Arc<cache::Cache>, email: &str) -> Option<i64> {
    let until = cache.get::<i64>(&lock_key(email)).await.ok().flatten()?;
    let now = now_epoch();
    if until > now {
        Some(until)
    } else {
        None
    }
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── Pre-login gate ──────────────────────────────────────────────────────

/// Decision returned by [`pre_login_check`].
#[derive(Debug, Clone)]
pub enum LoginGate {
    /// Account is locked; do not even attempt password verification.
    /// `unlock_at_epoch` is the absolute epoch second when it lifts.
    Locked { unlock_at_epoch: i64 },
    /// Caller MUST present a valid captcha; reject early if missing/wrong.
    RequireCaptcha,
    /// Proceed to password verification with no captcha gate.
    Allow,
}

/// Inspect the current state of `email` (lock + failure count) against the
/// configured thresholds and decide whether the login attempt may proceed.
pub async fn pre_login_check(
    ctx: &AppContext,
    email: &str,
    thresholds: &LoginThresholds,
) -> LoginGate {
    if let Some(unlock_at_epoch) = get_lock_until(&ctx.cache, email).await {
        return LoginGate::Locked { unlock_at_epoch };
    }
    if thresholds.captcha_required {
        return LoginGate::RequireCaptcha;
    }
    let fails = get_fail_count(&ctx.cache, email).await;
    if fails >= thresholds.max_failures_before_captcha {
        LoginGate::RequireCaptcha
    } else {
        LoginGate::Allow
    }
}

// ── Post-attempt mutators ───────────────────────────────────────────────

/// Increment the per-account failure counter. If the new count crosses the
/// lock threshold, set the lock key for `lock_duration_seconds`. Returns
/// the new failure count and whether captcha is now required for the next
/// attempt.
pub async fn record_failure(
    ctx: &AppContext,
    email: &str,
    thresholds: &LoginThresholds,
) -> (i64, bool, Option<i64>) {
    let key = fail_key(email);
    let new_count = get_fail_count(&ctx.cache, email).await + 1;

    // Re-set with the full window TTL on every failure (sliding window).
    let _ = ctx
        .cache
        .insert_with_expiry(
            &key,
            &new_count,
            Duration::from_secs(thresholds.failure_window_seconds.max(1) as u64),
        )
        .await;

    let lock_until: Option<i64> = if new_count >= thresholds.max_failures_before_lock {
        let until = now_epoch() + thresholds.lock_duration_seconds.max(1);
        let _ = ctx
            .cache
            .insert_with_expiry(
                &lock_key(email),
                &until,
                Duration::from_secs(thresholds.lock_duration_seconds.max(1) as u64),
            )
            .await;
        Some(until)
    } else {
        None
    };

    let captcha_required_next = new_count >= thresholds.max_failures_before_captcha;
    (new_count, captcha_required_next, lock_until)
}

/// Clear failure counter & lock on a successful login.
pub async fn record_success(ctx: &AppContext, email: &str) {
    let _ = ctx.cache.remove(&fail_key(email)).await;
    let _ = ctx.cache.remove(&lock_key(email)).await;
}

/// Administratively clear the lock and the sliding-window failure counter
/// for `email`. Functionally identical to [`record_success`] but exposed
/// under a name that conveys intent at the call-site (admin-driven unlock
/// vs. self-driven login success). Idempotent — calling it on a
/// not-locked account is a harmless no-op.
pub async fn unlock(ctx: &AppContext, email: &str) {
    record_success(ctx, email).await;
}
