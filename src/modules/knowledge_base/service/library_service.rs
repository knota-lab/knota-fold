use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder,
};
use uuid::Uuid;

use crate::models::_entities::{kb_documents, kb_folders, kb_libraries};
use crate::modules::knowledge_base::errors::KnowledgeBaseError;
use crate::modules::knowledge_base::models::{
    kb_folders as folder_models, kb_libraries as library_models,
};
use crate::utils::error::IntoAppError;
use crate::views::errors::err_bad_request;

#[derive(Debug)]
pub struct CreateLibraryParams {
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_by: Uuid,
}

#[derive(Debug)]
pub struct UpdateLibraryParams {
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug)]
pub struct CreateFolderParams {
    pub tenant_id: Uuid,
    pub library_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub sort_order: i32,
    pub created_by: Uuid,
}

#[derive(Debug)]
pub struct UpdateFolderParams {
    pub name: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct DocumentLocation {
    pub library_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
}

#[tracing::instrument(skip(db))]
pub async fn create_library(
    db: &DatabaseConnection,
    params: &CreateLibraryParams,
) -> loco_rs::Result<kb_libraries::Model> {
    let model = library_models::ActiveModel {
        tenant_id: ActiveValue::Set(params.tenant_id),
        name: ActiveValue::Set(params.name.clone()),
        description: ActiveValue::Set(params.description.clone()),
        sort_order: ActiveValue::Set(params.sort_order),
        created_by: ActiveValue::Set(params.created_by),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

#[tracing::instrument(skip(db))]
pub async fn list_libraries(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<kb_libraries::Model>> {
    kb_libraries::Entity::find()
        .filter(kb_libraries::Column::TenantId.eq(tenant_id))
        .order_by_asc(kb_libraries::Column::SortOrder)
        .order_by_asc(kb_libraries::Column::CreatedAt)
        .all(db)
        .await
        .db_err()
}

#[tracing::instrument(skip(db))]
pub async fn get_library(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    library_id: Uuid,
) -> loco_rs::Result<kb_libraries::Model> {
    kb_libraries::Entity::find_by_id(library_id)
        .filter(kb_libraries::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())
}

#[tracing::instrument(skip(db))]
pub async fn update_library(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    library_id: Uuid,
    params: &UpdateLibraryParams,
) -> loco_rs::Result<kb_libraries::Model> {
    let library = get_library(db, tenant_id, library_id).await?;
    let mut active: library_models::ActiveModel = library.into();
    active.name = ActiveValue::Set(params.name.clone());
    active.description = ActiveValue::Set(params.description.clone());
    active.sort_order = ActiveValue::Set(params.sort_order);
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()
}

#[tracing::instrument(skip(db))]
pub async fn delete_library(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    library_id: Uuid,
) -> loco_rs::Result<()> {
    let _library = get_library(db, tenant_id, library_id).await?;
    let folder_count = kb_folders::Entity::find()
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .filter(kb_folders::Column::LibraryId.eq(library_id))
        .count(db)
        .await
        .db_err()?;
    let document_count = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::LibraryId.eq(library_id))
        .count(db)
        .await
        .db_err()?;
    if folder_count > 0 || document_count > 0 {
        return Err(err_bad_request(
            "knowledge_base.library_not_empty",
            "library is not empty",
        ));
    }

    kb_libraries::Entity::delete_by_id(library_id)
        .exec(db)
        .await
        .db_err()?;
    Ok(())
}

#[tracing::instrument(skip(db))]
pub async fn create_folder(
    db: &DatabaseConnection,
    params: &CreateFolderParams,
) -> loco_rs::Result<kb_folders::Model> {
    let _library = get_library(db, params.tenant_id, params.library_id).await?;
    let (path_prefix, depth) = if let Some(parent_id) = params.parent_id {
        let parent = get_folder(db, params.tenant_id, parent_id).await?;
        if parent.library_id != params.library_id {
            return Err(err_bad_request(
                "knowledge_base.folder_library_mismatch",
                "parent folder does not belong to the target library",
            ));
        }
        (parent.path, parent.depth + 1)
    } else {
        ("/".to_string(), 0)
    };
    let id = Uuid::now_v7();
    let path = format!("{path_prefix}{id}/");
    let model = folder_models::ActiveModel {
        id: ActiveValue::Set(id),
        tenant_id: ActiveValue::Set(params.tenant_id),
        library_id: ActiveValue::Set(params.library_id),
        parent_id: ActiveValue::Set(params.parent_id),
        name: ActiveValue::Set(params.name.clone()),
        path: ActiveValue::Set(path),
        depth: ActiveValue::Set(depth),
        sort_order: ActiveValue::Set(params.sort_order),
        created_by: ActiveValue::Set(params.created_by),
        ..Default::default()
    };
    model.insert(db).await.db_err()
}

#[tracing::instrument(skip(db))]
pub async fn list_folders(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    library_id: Uuid,
    parent_id: Option<Uuid>,
) -> loco_rs::Result<Vec<kb_folders::Model>> {
    let _library = get_library(db, tenant_id, library_id).await?;
    let mut query = kb_folders::Entity::find()
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .filter(kb_folders::Column::LibraryId.eq(library_id));
    query = match parent_id {
        Some(id) => query.filter(kb_folders::Column::ParentId.eq(id)),
        None => query.filter(kb_folders::Column::ParentId.is_null()),
    };
    query
        .order_by_asc(kb_folders::Column::SortOrder)
        .order_by_asc(kb_folders::Column::CreatedAt)
        .all(db)
        .await
        .db_err()
}

#[tracing::instrument(skip(db))]
pub async fn get_folder(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    folder_id: Uuid,
) -> loco_rs::Result<kb_folders::Model> {
    kb_folders::Entity::find_by_id(folder_id)
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .db_err()?
        .ok_or_else(|| KnowledgeBaseError::NotFound.to_err())
}

#[tracing::instrument(skip(db))]
pub async fn list_folder_subtree_ids(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    folder_id: Uuid,
) -> loco_rs::Result<Vec<Uuid>> {
    let folder = get_folder(db, tenant_id, folder_id).await?;
    kb_folders::Entity::find()
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .filter(kb_folders::Column::LibraryId.eq(folder.library_id))
        .filter(kb_folders::Column::Path.starts_with(folder.path))
        .all(db)
        .await
        .db_err()
        .map(|folders| folders.into_iter().map(|item| item.id).collect())
}

#[tracing::instrument(skip(db))]
pub async fn update_folder(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    folder_id: Uuid,
    params: &UpdateFolderParams,
) -> loco_rs::Result<kb_folders::Model> {
    let folder = get_folder(db, tenant_id, folder_id).await?;
    let mut active: folder_models::ActiveModel = folder.into();
    active.name = ActiveValue::Set(params.name.clone());
    active.sort_order = ActiveValue::Set(params.sort_order);
    active.updated_at = ActiveValue::Set(Utc::now().naive_utc());
    active.update(db).await.db_err()
}

#[tracing::instrument(skip(db))]
pub async fn delete_folder(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    folder_id: Uuid,
) -> loco_rs::Result<()> {
    let folder = get_folder(db, tenant_id, folder_id).await?;
    let child_count = kb_folders::Entity::find()
        .filter(kb_folders::Column::TenantId.eq(tenant_id))
        .filter(kb_folders::Column::ParentId.eq(folder_id))
        .count(db)
        .await
        .db_err()?;
    let document_count = kb_documents::Entity::find()
        .filter(kb_documents::Column::TenantId.eq(tenant_id))
        .filter(kb_documents::Column::FolderId.eq(folder_id))
        .count(db)
        .await
        .db_err()?;
    if child_count > 0 || document_count > 0 {
        return Err(err_bad_request(
            "knowledge_base.folder_not_empty",
            "folder is not empty",
        ));
    }

    kb_folders::Entity::delete_by_id(folder.id)
        .exec(db)
        .await
        .db_err()?;
    Ok(())
}

#[tracing::instrument(skip(db))]
pub async fn resolve_document_location(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    library_id: Option<Uuid>,
    folder_id: Option<Uuid>,
) -> loco_rs::Result<DocumentLocation> {
    if let Some(folder_id) = folder_id {
        let folder = get_folder(db, tenant_id, folder_id).await?;
        if let Some(library_id) = library_id {
            if folder.library_id != library_id {
                return Err(err_bad_request(
                    "knowledge_base.document_location_mismatch",
                    "folder does not belong to the target library",
                ));
            }
        }
        return Ok(DocumentLocation {
            library_id: Some(folder.library_id),
            folder_id: Some(folder.id),
        });
    }

    if let Some(library_id) = library_id {
        let _library = get_library(db, tenant_id, library_id).await?;
    }
    Ok(DocumentLocation {
        library_id,
        folder_id: None,
    })
}
