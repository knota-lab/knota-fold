use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // 1. notifications table
        m.create_table(
            Table::create()
                .table(Notifications::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Notifications::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Notifications::TenantId).uuid().null())
                .col(
                    ColumnDef::new(Notifications::Title)
                        .string_len(256)
                        .not_null(),
                )
                .col(ColumnDef::new(Notifications::Content).text().not_null())
                .col(
                    ColumnDef::new(Notifications::NotificationType)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Notifications::Priority)
                        .string_len(16)
                        .not_null()
                        .default("normal"),
                )
                .col(ColumnDef::new(Notifications::CreatedBy).uuid().not_null())
                .col(ColumnDef::new(Notifications::TargetRoleCodes).text().null())
                .col(
                    ColumnDef::new(Notifications::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .col(
                    ColumnDef::new(Notifications::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Notifications::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 2. notification_recipients table
        m.create_table(
            Table::create()
                .table(NotificationRecipients::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(NotificationRecipients::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(NotificationRecipients::NotificationId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(NotificationRecipients::UserId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(NotificationRecipients::ReadAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationRecipients::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 3. Indexes
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_notifications_tenant_id ON notifications (tenant_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_notifications_created_by ON notifications (created_by)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_notifications_type_status ON notifications (notification_type, status)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_recipients_notification ON notification_recipients (notification_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_recipients_user_unread ON notification_recipients (user_id, read_at) WHERE read_at IS NULL",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_recipients_user_notification ON notification_recipients (user_id, notification_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(NotificationRecipients::Table)
                .to_owned(),
        )
        .await?;
        m.drop_table(Table::drop().table(Notifications::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum Notifications {
    Table,
    Id,
    TenantId,
    Title,
    Content,
    NotificationType,
    Priority,
    CreatedBy,
    TargetRoleCodes,
    Status,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum NotificationRecipients {
    Table,
    Id,
    NotificationId,
    UserId,
    ReadAt,
    CreatedAt,
}
