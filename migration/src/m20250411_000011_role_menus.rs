use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(RoleMenus::Table)
                .if_not_exists()
                .col(ColumnDef::new(RoleMenus::TenantId).uuid().not_null())
                .col(ColumnDef::new(RoleMenus::RoleId).uuid().not_null())
                .col(ColumnDef::new(RoleMenus::SysMenuId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(RoleMenus::TenantId)
                        .col(RoleMenus::RoleId)
                        .col(RoleMenus::SysMenuId),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_role_menus_tenant_role")
                .table(RoleMenus::Table)
                .col(RoleMenus::TenantId)
                .col(RoleMenus::RoleId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(RoleMenus::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum RoleMenus {
    Table,
    TenantId,
    RoleId,
    SysMenuId,
}
