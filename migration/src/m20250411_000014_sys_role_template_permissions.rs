use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(SysRoleTemplatePermissions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(SysRoleTemplatePermissions::TemplateId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SysRoleTemplatePermissions::Obj)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SysRoleTemplatePermissions::Act)
                        .string_len(32)
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .col(SysRoleTemplatePermissions::TemplateId)
                        .col(SysRoleTemplatePermissions::Obj)
                        .col(SysRoleTemplatePermissions::Act),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_sys_role_template_permissions_template_id")
                .table(SysRoleTemplatePermissions::Table)
                .col(SysRoleTemplatePermissions::TemplateId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(SysRoleTemplatePermissions::Table)
                .to_owned(),
        )
        .await
    }
}

#[derive(Iden)]
enum SysRoleTemplatePermissions {
    Table,
    TemplateId,
    Obj,
    Act,
}
