use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Permissions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Permissions::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Permissions::Name).string_len(64).not_null())
                .col(ColumnDef::new(Permissions::Code).string_len(128).not_null())
                .col(ColumnDef::new(Permissions::Obj).string_len(128).not_null())
                .col(ColumnDef::new(Permissions::Act).string_len(32).not_null())
                .col(
                    ColumnDef::new(Permissions::PermType)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Permissions::IsSystem)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(Permissions::Version)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(Permissions::UpdatedBy).uuid().null())
                .col(
                    ColumnDef::new(Permissions::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Permissions::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Permissions::DeletedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uk_permissions_code")
                .table(Permissions::Table)
                .col(Permissions::Code)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("uk_permissions_obj_act")
                .table(Permissions::Table)
                .col(Permissions::Obj)
                .col(Permissions::Act)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_permissions_deleted_at")
                .table(Permissions::Table)
                .col(Permissions::DeletedAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Permissions::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum Permissions {
    Table,
    Id,
    Name,
    Code,
    Obj,
    Act,
    #[iden = "type"]
    PermType,
    IsSystem,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}
