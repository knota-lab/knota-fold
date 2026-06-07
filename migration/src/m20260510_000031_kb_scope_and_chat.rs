use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // 1. chat_sessions table
        m.create_table(
            Table::create()
                .table(ChatSessions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ChatSessions::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(ChatSessions::TenantId).uuid().not_null())
                .col(ColumnDef::new(ChatSessions::UserId).uuid().not_null())
                .col(ColumnDef::new(ChatSessions::Title).string_len(512).null())
                .col(
                    ColumnDef::new(ChatSessions::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(ChatSessions::UpdatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 2. chat_messages table
        m.create_table(
            Table::create()
                .table(ChatMessages::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ChatMessages::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(ChatMessages::SessionId).uuid().not_null())
                .col(ColumnDef::new(ChatMessages::TenantId).uuid().not_null())
                .col(ColumnDef::new(ChatMessages::UserId).uuid().not_null())
                .col(ColumnDef::new(ChatMessages::Role).string_len(16).not_null())
                .col(ColumnDef::new(ChatMessages::Content).text().not_null())
                .col(
                    ColumnDef::new(ChatMessages::MaterialRefs)
                        .json_binary()
                        .null(),
                )
                .col(ColumnDef::new(ChatMessages::Intent).string_len(32).null())
                .col(ColumnDef::new(ChatMessages::Strategy).string_len(32).null())
                .col(
                    ColumnDef::new(ChatMessages::TokenUsage)
                        .json_binary()
                        .null(),
                )
                .col(
                    ColumnDef::new(ChatMessages::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 3. Indexes via raw SQL
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_chat_sessions_user ON chat_sessions (tenant_id, user_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_chat_messages_session ON chat_messages (session_id, created_at)",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // 1. Drop chat_messages
        m.drop_table(Table::drop().table(ChatMessages::Table).to_owned())
            .await?;

        // 2. Drop chat_sessions
        m.drop_table(Table::drop().table(ChatSessions::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(Iden)]
enum ChatSessions {
    Table,
    Id,
    TenantId,
    UserId,
    Title,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum ChatMessages {
    Table,
    Id,
    SessionId,
    TenantId,
    UserId,
    Role,
    Content,
    MaterialRefs,
    Intent,
    Strategy,
    TokenUsage,
    CreatedAt,
}
