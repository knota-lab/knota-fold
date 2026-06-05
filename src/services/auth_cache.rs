//! Thin caching layer over auth-related DB lookups.
//!
//! Two cache keys per user:
//!   - `user:pwd_iat:{user_id}`  → `i64` (epoch seconds of `password_changed_at`)
//!   - `user:profile:{user_id}`  → serialised [`CachedUserProfile`]
//!
//! All values have a short TTL so stale data self-heals even without explicit
//! invalidation.  Explicit invalidation is done on write paths (password change,
//! profile update) for immediate consistency.
//!
//! The cache backend is selected by `config/*.yaml` (`InMem` / `Redis`);
//! this module is backend-agnostic via `loco_rs::cache::Cache`.

use std::sync::Arc;
use std::time::Duration;

use loco_rs::cache;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::users;
use crate::utils::error::OptionErrInto;

// ── TTLs ────────────────────────────────────────────────────────────────

/// How long a `password_changed_at` timestamp stays cached before re-fetch.
const PWD_IAT_TTL: Duration = Duration::from_mins(1);

/// How long a full user profile stays cached.
const PROFILE_TTL: Duration = Duration::from_mins(5);

// ── Cache key helpers ───────────────────────────────────────────────────

fn pwd_iat_key(user_id: Uuid) -> String {
    format!("user:pwd_iat:{user_id}")
}

fn profile_key(user_id: Uuid) -> String {
    format!("user:profile:{user_id}")
}

// ── Cached types ────────────────────────────────────────────────────────

/// Minimal user record cached for `/auth/current` and middleware checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedUserProfile {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub name: String,
    pub status: String,
    pub avatar_file_id: Option<Uuid>,
    pub password_changed_at_epoch: i64,
}

impl CachedUserProfile {
    #[must_use]
    pub fn from_model(m: &users::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            email: m.email.clone(),
            name: m.name.clone(),
            status: m.status.clone(),
            avatar_file_id: m.avatar_file_id,
            password_changed_at_epoch: m
                .password_changed_at
                .unwrap_or(m.created_at)
                .timestamp(),
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────

/// Get the epoch-seconds timestamp of the user's last password change.
/// Cache-first; falls back to DB on miss.
pub async fn get_password_iat(
    cache: &Arc<cache::Cache>,
    db: &DatabaseConnection,
    user_id: Uuid,
) -> loco_rs::Result<i64> {
    let key = pwd_iat_key(user_id);

    // Try cache first
    if let Ok(Some(ts)) = cache.get::<i64>(&key).await {
        return Ok(ts);
    }

    // Cache miss → DB
    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let ts = user
        .password_changed_at
        .unwrap_or(user.created_at)
        .timestamp();
    let _ = cache.insert_with_expiry(&key, &ts, PWD_IAT_TTL).await;
    Ok(ts)
}

/// Get the full cached user profile (for `/auth/current` and middleware).
/// Cache-first; falls back to DB on miss.
pub async fn get_user_profile(
    cache: &Arc<cache::Cache>,
    db: &DatabaseConnection,
    user_id: Uuid,
) -> loco_rs::Result<CachedUserProfile> {
    let key = profile_key(user_id);

    if let Ok(Some(profile)) = cache.get::<CachedUserProfile>(&key).await {
        return Ok(profile);
    }

    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let profile = CachedUserProfile::from_model(&user);
    let _ = cache.insert_with_expiry(&key, &profile, PROFILE_TTL).await;
    Ok(profile)
}

/// Invalidate all auth caches for a user (call on password change, profile
/// update, status toggle, etc.).
pub async fn invalidate_user(cache: &Arc<cache::Cache>, user_id: Uuid) {
    let _ = cache.remove(&pwd_iat_key(user_id)).await;
    let _ = cache.remove(&profile_key(user_id)).await;
}
