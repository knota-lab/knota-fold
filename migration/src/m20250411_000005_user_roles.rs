use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(UserRoles::Table)
                .if_not_exists()
                .col(ColumnDef::new(UserRoles::TenantId).uuid().not_null())
                .col(ColumnDef::new(UserRoles::UserId).uuid().not_null())
                .col(ColumnDef::new(UserRoles::RoleId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(UserRoles::TenantId)
                        .col(UserRoles::UserId)
                        .col(UserRoles::RoleId),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_user_roles_user_id")
                .table(UserRoles::Table)
                .col(UserRoles::UserId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_user_roles_role_id")
                .table(UserRoles::Table)
                .col(UserRoles::RoleId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_user_roles_user_tenant")
                .table(UserRoles::Table)
                .col(UserRoles::UserId)
                .col(UserRoles::TenantId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(UserRoles::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum UserRoles {
    Table,
    TenantId,
    UserId,
    RoleId,
}
