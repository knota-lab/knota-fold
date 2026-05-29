//! `SeaORM` Entity, generated for Wave 4 instant-upload idempotency cache.
//!
//! Distinct from `file_upload_idempotency` because the instant-upload path
//! does not allocate a `file_uploads` row, so the idempotency key is scoped
//! by `(tenant_id, user_id, expected_hash, idempotency_key)` instead.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "file_instant_idempotency")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub expected_hash: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub idempotency_key: String,
    pub response_body: Vec<u8>,
    pub status_code: i32,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
