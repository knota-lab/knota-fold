use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(AuditLogs::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(AuditLogs::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(AuditLogs::RequestId).string_len(64).null())
                .col(ColumnDef::new(AuditLogs::TraceId).string_len(36).null())
                .col(ColumnDef::new(AuditLogs::TenantId).uuid().not_null())
                .col(ColumnDef::new(AuditLogs::UserId).uuid().null())
                .col(ColumnDef::new(AuditLogs::Action).string_len(16).not_null())
                .col(
                    ColumnDef::new(AuditLogs::ResourceType)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(AuditLogs::ResourceId)
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(AuditLogs::BeforeState).json().null())
                .col(ColumnDef::new(AuditLogs::AfterState).json().null())
                .col(ColumnDef::new(AuditLogs::IpAddress).string_len(45).null())
                .col(ColumnDef::new(AuditLogs::UserAgent).string_len(512).null())
                .col(
                    ColumnDef::new(AuditLogs::Status)
                        .string_len(16)
                        .not_null()
                        .default("success"),
                )
                .col(ColumnDef::new(AuditLogs::ErrorMessage).text().null())
                .col(
                    ColumnDef::new(AuditLogs::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // idx: tenant + created_at DESC (admin views tenant audit history)
        m.create_index(
            Index::create()
                .name("idx_audit_tenant_created")
                .table(AuditLogs::Table)
                .col(AuditLogs::TenantId)
                .col(AuditLogs::CreatedAt)
                .to_owned(),
        )
        .await?;

        // idx: resource tracing
        m.create_index(
            Index::create()
                .name("idx_audit_resource")
                .table(AuditLogs::Table)
                .col(AuditLogs::ResourceType)
                .col(AuditLogs::ResourceId)
                .col(AuditLogs::CreatedAt)
                .to_owned(),
        )
        .await?;

        // idx: by operator
        m.create_index(
            Index::create()
                .name("idx_audit_user_created")
                .table(AuditLogs::Table)
                .col(AuditLogs::UserId)
                .col(AuditLogs::CreatedAt)
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(AuditLogs::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum AuditLogs {
    Table,
    Id,
    RequestId,
    TraceId,
    TenantId,
    UserId,
    Action,
    ResourceType,
    ResourceId,
    BeforeState,
    AfterState,
    IpAddress,
    UserAgent,
    Status,
    ErrorMessage,
    CreatedAt,
}
