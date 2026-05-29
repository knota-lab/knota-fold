use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(SysRoleTemplates::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(SysRoleTemplates::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(SysRoleTemplates::Code)
                        .string_len(64)
                        .not_null()
                        .unique_key(),
                )
                .col(
                    ColumnDef::new(SysRoleTemplates::Name)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(SysRoleTemplates::Description).text().null())
                .col(
                    ColumnDef::new(SysRoleTemplates::IsDefault)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(SysRoleTemplates::SortOrder)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(SysRoleTemplates::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(SysRoleTemplates::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(SysRoleTemplates::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum SysRoleTemplates {
    Table,
    Id,
    Code,
    Name,
    Description,
    IsDefault,
    SortOrder,
    CreatedAt,
    UpdatedAt,
}
