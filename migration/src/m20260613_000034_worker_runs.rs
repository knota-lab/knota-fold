use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(WorkerRuns::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(WorkerRuns::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(WorkerRuns::TenantId).uuid().null())
                .col(
                    ColumnDef::new(WorkerRuns::WorkerName)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(WorkerRuns::BusinessType)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(WorkerRuns::BusinessId)
                        .string_len(128)
                        .not_null(),
                )
                .col(ColumnDef::new(WorkerRuns::Status).string_len(16).not_null())
                .col(ColumnDef::new(WorkerRuns::Stage).string_len(64).null())
                .col(
                    ColumnDef::new(WorkerRuns::StageLabel)
                        .string_len(128)
                        .null(),
                )
                .col(ColumnDef::new(WorkerRuns::Message).text().null())
                .col(ColumnDef::new(WorkerRuns::Current).integer().null())
                .col(ColumnDef::new(WorkerRuns::Total).integer().null())
                .col(
                    ColumnDef::new(WorkerRuns::Attempt)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(WorkerRuns::HeartbeatAt).date_time().null())
                .col(
                    ColumnDef::new(WorkerRuns::StageStartedAt)
                        .date_time()
                        .null(),
                )
                .col(ColumnDef::new(WorkerRuns::StartedAt).date_time().null())
                .col(ColumnDef::new(WorkerRuns::FinishedAt).date_time().null())
                .col(ColumnDef::new(WorkerRuns::DurationMs).big_integer().null())
                .col(ColumnDef::new(WorkerRuns::ErrorCode).string_len(128).null())
                .col(ColumnDef::new(WorkerRuns::ErrorMessage).text().null())
                .col(ColumnDef::new(WorkerRuns::TraceId).string_len(64).null())
                .col(ColumnDef::new(WorkerRuns::Metadata).json_binary().null())
                .col(
                    ColumnDef::new(WorkerRuns::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(WorkerRuns::UpdatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_tenant_id ON worker_runs (tenant_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_worker_name ON worker_runs (worker_name)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_business ON worker_runs (business_type, business_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_status ON worker_runs (status)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_heartbeat_at ON worker_runs (heartbeat_at)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_runs_created_at ON worker_runs (created_at)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(WorkerRuns::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum WorkerRuns {
    Table,
    Id,
    TenantId,
    WorkerName,
    BusinessType,
    BusinessId,
    Status,
    Stage,
    StageLabel,
    Message,
    Current,
    Total,
    Attempt,
    HeartbeatAt,
    StageStartedAt,
    StartedAt,
    FinishedAt,
    DurationMs,
    ErrorCode,
    ErrorMessage,
    TraceId,
    Metadata,
    CreatedAt,
    UpdatedAt,
}
