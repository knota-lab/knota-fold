use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(ScheduledWorkerDefinitions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Code)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Name)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Description)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Category)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::ParamsSchema)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::TimeoutSecs)
                        .integer()
                        .not_null()
                        .default(300i32),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::MaxRetries)
                        .integer()
                        .not_null()
                        .default(3i32),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::AllowConcurrent)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::IsSystem)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Status)
                        .string_len(16)
                        .not_null()
                        .default("active"),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::Version)
                        .integer()
                        .not_null()
                        .default(1i32),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::UpdatedBy)
                        .uuid()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerDefinitions::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_scheduled_worker_definitions_code \
                 ON scheduled_worker_definitions (code)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_definitions_category \
                 ON scheduled_worker_definitions (category)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_definitions_status \
                 ON scheduled_worker_definitions (status)",
            )
            .await?;

        m.create_table(
            Table::create()
                .table(ScheduledWorkerTenantGrants::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ScheduledWorkerTenantGrants::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerTenantGrants::WorkerDefId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerTenantGrants::TenantId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerTenantGrants::GrantedBy)
                        .uuid()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerTenantGrants::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_scheduled_worker_tenant_grants_worker_def_tenant \
                 ON scheduled_worker_tenant_grants (worker_def_id, tenant_id)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_tenant_grants_worker_def_id \
                 ON scheduled_worker_tenant_grants (worker_def_id)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_tenant_grants_tenant_id \
                 ON scheduled_worker_tenant_grants (tenant_id)",
            )
            .await?;

        m.create_table(
            Table::create()
                .table(ScheduledWorkerSchedules::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::WorkerDefId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::TenantId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::Name)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::CronExpr)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::ParamsJson)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::Enabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::LastRunAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::NextRunAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::CreatedBy)
                        .uuid()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::UpdatedBy)
                        .uuid()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerSchedules::UpdatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_scheduled_worker_schedules_worker_def_tenant_name \
                 ON scheduled_worker_schedules (worker_def_id, tenant_id, name)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_schedules_tenant_id \
                 ON scheduled_worker_schedules (tenant_id)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_worker_schedules_next \
                 ON scheduled_worker_schedules (next_run_at) WHERE enabled",
            )
            .await?;

        m.create_table(
            Table::create()
                .table(ScheduledWorkerExecutions::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::ScheduleId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::WorkerDefId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::TenantId)
                        .uuid()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::TriggerType)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::TriggeredBy)
                        .uuid()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::ParamsJson)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::Status)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::RetryCount)
                        .integer()
                        .not_null()
                        .default(0i32),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::StartedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::FinishedAt)
                        .timestamp_with_time_zone()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::DurationMs)
                        .integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::Output)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::ErrorMessage)
                        .text()
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::Traceparent)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::ParentSpanId)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(ScheduledWorkerExecutions::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_executions_schedule_id \
                 ON scheduled_worker_executions (schedule_id)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_executions_tenant_id \
                 ON scheduled_worker_executions (tenant_id)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_executions_status \
                 ON scheduled_worker_executions (status)",
            )
            .await?;

        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_scheduled_worker_executions_created_at \
                 ON scheduled_worker_executions (created_at)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(ScheduledWorkerExecutions::Table)
                .to_owned(),
        )
        .await?;
        m.drop_table(
            Table::drop()
                .table(ScheduledWorkerSchedules::Table)
                .to_owned(),
        )
        .await?;
        m.drop_table(
            Table::drop()
                .table(ScheduledWorkerTenantGrants::Table)
                .to_owned(),
        )
        .await?;
        m.drop_table(
            Table::drop()
                .table(ScheduledWorkerDefinitions::Table)
                .to_owned(),
        )
        .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum ScheduledWorkerDefinitions {
    Table,
    Id,
    Code,
    Name,
    Description,
    Category,
    ParamsSchema,
    TimeoutSecs,
    MaxRetries,
    AllowConcurrent,
    IsSystem,
    Status,
    Version,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum ScheduledWorkerTenantGrants {
    Table,
    Id,
    WorkerDefId,
    TenantId,
    GrantedBy,
    CreatedAt,
}

#[derive(Iden)]
enum ScheduledWorkerSchedules {
    Table,
    Id,
    WorkerDefId,
    TenantId,
    Name,
    CronExpr,
    ParamsJson,
    Enabled,
    LastRunAt,
    NextRunAt,
    CreatedBy,
    UpdatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum ScheduledWorkerExecutions {
    Table,
    Id,
    ScheduleId,
    WorkerDefId,
    TenantId,
    TriggerType,
    TriggeredBy,
    ParamsJson,
    Status,
    RetryCount,
    StartedAt,
    FinishedAt,
    DurationMs,
    Output,
    ErrorMessage,
    Traceparent,
    ParentSpanId,
    CreatedAt,
}
