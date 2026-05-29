use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(SysRoleTemplateMenus::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(SysRoleTemplateMenus::TemplateId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(SysRoleTemplateMenus::SysMenuId)
                        .uuid()
                        .not_null(),
                )
                .primary_key(
                    Index::create()
                        .col(SysRoleTemplateMenus::TemplateId)
                        .col(SysRoleTemplateMenus::SysMenuId),
                )
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_sys_role_template_menus_template_id")
                .table(SysRoleTemplateMenus::Table)
                .col(SysRoleTemplateMenus::TemplateId)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(SysRoleTemplateMenus::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum SysRoleTemplateMenus {
    Table,
    TemplateId,
    SysMenuId,
}
