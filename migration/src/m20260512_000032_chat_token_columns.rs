use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.get_connection()
            .execute_unprepared(
                "ALTER TABLE chat_messages ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "ALTER TABLE chat_messages ADD COLUMN completion_tokens INTEGER NOT NULL DEFAULT 0",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "ALTER TABLE chat_messages ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.get_connection()
            .execute_unprepared("ALTER TABLE chat_messages DROP COLUMN total_tokens")
            .await?;

        m.get_connection()
            .execute_unprepared("ALTER TABLE chat_messages DROP COLUMN completion_tokens")
            .await?;

        m.get_connection()
            .execute_unprepared("ALTER TABLE chat_messages DROP COLUMN prompt_tokens")
            .await?;

        Ok(())
    }
}
