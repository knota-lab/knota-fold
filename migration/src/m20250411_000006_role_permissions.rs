use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(RolePermissions::Table)
                .if_not_exists()
                .col(ColumnDef::new(RolePermissions::TenantId).uuid().not_null())
                .col(ColumnDef::new(RolePermissions::RoleId).uuid().not_null())
                .col(
                    ColumnDef::new(RolePermissions::PermissionId)
                        .uuid()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .col(RolePermissions::TenantId)
                        .col(RolePermissions::RoleId)
                        .col(RolePermissions::PermissionId),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_role_permissions_role_id")
                .table(RolePermissions::Table)
                .col(RolePermissions::RoleId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_role_permissions_permission_id")
                .table(RolePermissions::Table)
                .col(RolePermissions::PermissionId)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_role_permissions_tenant_id")
                .table(RolePermissions::Table)
                .col(RolePermissions::TenantId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(RolePermissions::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum RolePermissions {
    Table,
    TenantId,
    RoleId,
    PermissionId,
}
