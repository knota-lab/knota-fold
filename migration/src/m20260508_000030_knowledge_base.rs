use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        // 1. kb_libraries table
        m.create_table(
            Table::create()
                .table(KbLibraries::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(KbLibraries::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(KbLibraries::TenantId).uuid().not_null())
                .col(ColumnDef::new(KbLibraries::Name).string_len(128).not_null())
                .col(ColumnDef::new(KbLibraries::Description).text().null())
                .col(
                    ColumnDef::new(KbLibraries::SortOrder)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(ColumnDef::new(KbLibraries::CreatedBy).uuid().not_null())
                .col(
                    ColumnDef::new(KbLibraries::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(KbLibraries::UpdatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 2. kb_folders table
        m.create_table(
            Table::create()
                .table(KbFolders::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(KbFolders::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(KbFolders::TenantId).uuid().not_null())
                .col(ColumnDef::new(KbFolders::LibraryId).uuid().not_null())
                .col(ColumnDef::new(KbFolders::ParentId).uuid().null())
                .col(ColumnDef::new(KbFolders::Name).string_len(128).not_null())
                .col(ColumnDef::new(KbFolders::Path).text().not_null())
                .col(ColumnDef::new(KbFolders::Depth).integer().not_null())
                .col(
                    ColumnDef::new(KbFolders::SortOrder)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(ColumnDef::new(KbFolders::CreatedBy).uuid().not_null())
                .col(
                    ColumnDef::new(KbFolders::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(KbFolders::UpdatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 3. kb_documents table
        m.create_table(
            Table::create()
                .table(KbDocuments::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(KbDocuments::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(KbDocuments::TenantId).uuid().not_null())
                .col(ColumnDef::new(KbDocuments::LibraryId).uuid().null())
                .col(ColumnDef::new(KbDocuments::FolderId).uuid().null())
                .col(
                    ColumnDef::new(KbDocuments::Title)
                        .string_len(512)
                        .not_null(),
                )
                .col(ColumnDef::new(KbDocuments::Description).text().null())
                .col(
                    ColumnDef::new(KbDocuments::SourceType)
                        .string_len(32)
                        .not_null(),
                )
                .col(ColumnDef::new(KbDocuments::FileId).uuid().null())
                .col(ColumnDef::new(KbDocuments::FullText).text().null())
                .col(
                    ColumnDef::new(KbDocuments::Scope)
                        .string_len(16)
                        .not_null()
                        .default("tenant"),
                )
                .col(
                    ColumnDef::new(KbDocuments::Status)
                        .string_len(16)
                        .not_null()
                        .default("pending"),
                )
                .col(
                    ColumnDef::new(KbDocuments::ChunkCount)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(KbDocuments::TotalTokens)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(ColumnDef::new(KbDocuments::Metadata).json_binary().null())
                .col(ColumnDef::new(KbDocuments::ErrorMessage).text().null())
                .col(ColumnDef::new(KbDocuments::CreatedBy).uuid().not_null())
                .col(
                    ColumnDef::new(KbDocuments::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(KbDocuments::UpdatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 4. kb_chunks table
        m.create_table(
            Table::create()
                .table(KbChunks::Table)
                .if_not_exists()
                .col(ColumnDef::new(KbChunks::Id).uuid().not_null().primary_key())
                .col(ColumnDef::new(KbChunks::DocumentId).uuid().not_null())
                .col(ColumnDef::new(KbChunks::TenantId).uuid().not_null())
                .col(ColumnDef::new(KbChunks::ChunkIndex).integer().not_null())
                .col(ColumnDef::new(KbChunks::Content).text().not_null())
                .col(ColumnDef::new(KbChunks::HeadingPath).text().null())
                .col(ColumnDef::new(KbChunks::PageNumber).integer().null())
                .col(ColumnDef::new(KbChunks::TokenCount).integer().not_null())
                .col(ColumnDef::new(KbChunks::CharStart).integer().null())
                .col(ColumnDef::new(KbChunks::CharEnd).integer().null())
                .col(
                    ColumnDef::new(KbChunks::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await?;

        // 5. Unique index on kb_chunks(document_id, chunk_index)
        m.create_index(
            Index::create()
                .if_not_exists()
                .name("idx_kb_chunks_doc_chunk_unique")
                .table(KbChunks::Table)
                .col(KbChunks::DocumentId)
                .col(KbChunks::ChunkIndex)
                .unique()
                .to_owned(),
        )
        .await?;

        // 6. document_lines table
        //    Composite PK order: (tenant_id, document_id, line_number)
        //    matches the design doc and the window index prefix.
        m.create_table(
            Table::create()
                .table(DocumentLines::Table)
                .if_not_exists()
                .col(ColumnDef::new(DocumentLines::TenantId).uuid().not_null())
                .col(ColumnDef::new(DocumentLines::DocumentId).uuid().not_null())
                .col(
                    ColumnDef::new(DocumentLines::LineNumber)
                        .integer()
                        .not_null(),
                )
                .col(ColumnDef::new(DocumentLines::LineText).text().not_null())
                .col(
                    ColumnDef::new(DocumentLines::LineChars)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DocumentLines::CumulativeChars)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DocumentLines::CreatedAt)
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .primary_key(
                    Index::create()
                        .col(DocumentLines::TenantId)
                        .col(DocumentLines::DocumentId)
                        .col(DocumentLines::LineNumber),
                )
                .to_owned(),
        )
        .await?;

        // 7. Indexes
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_libraries_tenant ON kb_libraries (tenant_id, sort_order, created_at)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_kb_libraries_name_unique ON kb_libraries (tenant_id, name)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_folders_library_parent ON kb_folders (tenant_id, library_id, parent_id, sort_order, created_at)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_kb_folders_name_unique ON kb_folders (tenant_id, library_id, parent_id, name)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_folders_path ON kb_folders (tenant_id, library_id, path)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_docs_tenant ON kb_documents (tenant_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_docs_status ON kb_documents (tenant_id, status)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_docs_scope ON kb_documents (tenant_id, scope, created_by)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_docs_library_folder ON kb_documents (tenant_id, library_id, folder_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_chunks_doc ON kb_chunks (document_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_kb_chunks_tenant ON kb_chunks (tenant_id)",
            )
            .await?;
        m.get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_doclines_window ON document_lines (tenant_id, document_id, cumulative_chars)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(DocumentLines::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(KbChunks::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(KbDocuments::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(KbFolders::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(KbLibraries::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum KbLibraries {
    Table,
    Id,
    TenantId,
    Name,
    Description,
    SortOrder,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum KbFolders {
    Table,
    Id,
    TenantId,
    LibraryId,
    ParentId,
    Name,
    Path,
    Depth,
    SortOrder,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum KbDocuments {
    Table,
    Id,
    TenantId,
    LibraryId,
    FolderId,
    Title,
    Description,
    SourceType,
    FileId,
    FullText,
    Scope,
    Status,
    ChunkCount,
    TotalTokens,
    Metadata,
    ErrorMessage,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum KbChunks {
    Table,
    Id,
    DocumentId,
    TenantId,
    ChunkIndex,
    Content,
    HeadingPath,
    PageNumber,
    TokenCount,
    CharStart,
    CharEnd,
    CreatedAt,
}

#[derive(Iden)]
enum DocumentLines {
    Table,
    DocumentId,
    TenantId,
    LineNumber,
    LineText,
    LineChars,
    CumulativeChars,
    CreatedAt,
}
