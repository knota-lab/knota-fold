//! `SeaORM` Entity, generated for Wave 2b multipart idempotency cache

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "file_upload_idempotency")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub upload_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub endpoint: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub idempotency_key: String,
    pub response_body: Vec<u8>,
    pub status_code: i32,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
