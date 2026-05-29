use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::extractors::request_meta::RequestMeta;
use crate::extractors::TenantContext;
use crate::models::_entities::{
    audit_logs, dict_items, dict_types, file_references, file_uploads, files, roles,
    sys_menus, sys_role_templates, tenant_menu_overrides, tenants, users,
};

// ---------------------------------------------------------------------------
// AuditContext — assembled by controller from TenantContext + RequestMeta
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct AuditContext {
    pub trace_id: Option<String>,
    pub request_id: Option<String>,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

impl AuditContext {
    pub fn from_request(tc: &TenantContext, meta: &RequestMeta) -> Self {
        Self {
            trace_id: Some(meta.trace_id.clone()),
            request_id: meta.request_id.clone(),
            tenant_id: tc.tenant_id,
            user_id: Some(tc.user_id),
            ip_address: meta.ip_address.clone(),
            user_agent: meta.user_agent.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// AuditAction
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum AuditAction {
    Create,
    Update,
    Delete,
    ResetPassword,
    // File domain (设计文档 §13.2)
    UploadComplete,
    UploadAbort,
    SoftDelete,
    Restore,
    Reference,
    Dereference,
    Purge,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::ResetPassword => "reset_password",
            Self::UploadComplete => "upload_complete",
            Self::UploadAbort => "upload_abort",
            Self::SoftDelete => "soft_delete",
            Self::Restore => "restore",
            Self::Reference => "reference",
            Self::Dereference => "dereference",
            Self::Purge => "purge",
        }
    }
}

// ---------------------------------------------------------------------------
// AuditEntry — for batch operations
// ---------------------------------------------------------------------------

pub struct AuditEntry {
    pub action: AuditAction,
    pub resource_type: String,
    pub resource_id: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Audit Snapshots — explicit DTOs excluding sensitive fields
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAuditSnapshot {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub status: String,
    pub tenant_id: Uuid,
}

impl From<&users::Model> for UserAuditSnapshot {
    fn from(m: &users::Model) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            email: m.email.clone(),
            status: m.status.clone(),
            tenant_id: m.tenant_id,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAuditSnapshot {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub tenant_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub is_system: bool,
    pub description: Option<String>,
    pub status: String,
    pub version: i32,
}

impl From<&roles::Model> for RoleAuditSnapshot {
    fn from(m: &roles::Model) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            code: m.code.clone(),
            tenant_id: m.tenant_id,
            parent_id: m.parent_id,
            is_system: m.is_system,
            description: m.description.clone(),
            status: m.status.clone(),
            version: m.version,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantAuditSnapshot {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub status: String,
    pub description: Option<String>,
}

impl From<&tenants::Model> for TenantAuditSnapshot {
    fn from(m: &tenants::Model) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            code: m.code.clone(),
            status: m.status.clone(),
            description: m.description.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysMenuAuditSnapshot {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub path: Option<String>,
    pub icon: Option<String>,
    pub menu_type: String,
    pub sort_order: i32,
    pub status: String,
    pub parent_id: Option<Uuid>,
}

impl From<&sys_menus::Model> for SysMenuAuditSnapshot {
    fn from(m: &sys_menus::Model) -> Self {
        Self {
            id: m.id,
            code: m.code.clone(),
            name: m.name.clone(),
            path: m.path.clone(),
            icon: m.icon.clone(),
            menu_type: m.menu_type.clone(),
            sort_order: m.sort_order,
            status: m.status.clone(),
            parent_id: m.parent_id,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictTypeAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub source_type_id: Option<Uuid>,
    pub code: String,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub version: i32,
}

impl From<&dict_types::Model> for DictTypeAuditSnapshot {
    fn from(m: &dict_types::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            source_type_id: m.source_type_id,
            code: m.code.clone(),
            name: m.name.clone(),
            status: m.status.clone(),
            description: m.description.clone(),
            version: m.version,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictItemAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub dict_type_id: Uuid,
    pub source_item_id: Option<Uuid>,
    pub code: String,
    pub name: String,
    pub value: String,
    pub parent_id: Option<Uuid>,
    pub sort_order: i32,
    pub status: String,
    pub description: Option<String>,
    pub version: i32,
}

impl From<&dict_items::Model> for DictItemAuditSnapshot {
    fn from(m: &dict_items::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            dict_type_id: m.dict_type_id,
            source_item_id: m.source_item_id,
            code: m.code.clone(),
            name: m.name.clone(),
            value: m.value.clone(),
            parent_id: m.parent_id,
            sort_order: m.sort_order,
            status: m.status.clone(),
            description: m.description.clone(),
            version: m.version,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleTemplateAuditSnapshot {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub sort_order: i32,
}

impl From<&sys_role_templates::Model> for RoleTemplateAuditSnapshot {
    fn from(m: &sys_role_templates::Model) -> Self {
        Self {
            id: m.id,
            code: m.code.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            is_default: m.is_default,
            sort_order: m.sort_order,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantMenuOverrideAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub sys_menu_id: Uuid,
    pub custom_name: Option<String>,
    pub custom_icon: Option<String>,
    pub custom_sort: Option<i32>,
    pub is_hidden: bool,
}

impl From<&tenant_menu_overrides::Model> for TenantMenuOverrideAuditSnapshot {
    fn from(m: &tenant_menu_overrides::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            sys_menu_id: m.sys_menu_id,
            custom_name: m.custom_name.clone(),
            custom_icon: m.custom_icon.clone(),
            custom_sort: m.custom_sort,
            is_hidden: m.is_hidden,
        }
    }
}

// ---------------------------------------------------------------------------
// File / FileUpload Audit Snapshots
// SECURITY: Excludes sensitive storage paths (bucket / storage_key /
// multipart_upload_id / temp_key / s3_upload_id) per 文件管理.md §6.1 L297-298.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub content_hash: String,
    pub status: String,
    pub uploaded_by: Uuid,
    pub created_by: Uuid,
}

impl From<&files::Model> for FileAuditSnapshot {
    fn from(m: &files::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            name: m.name.clone(),
            mime_type: m.mime_type.clone(),
            size: m.size,
            content_hash: m.content_hash.clone(),
            status: m.status.clone(),
            uploaded_by: m.uploaded_by,
            created_by: m.created_by,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub file_name: String,
    pub expected_size: i64,
    pub status: String,
    pub parts_total: i32,
    pub parts_received: i32,
    pub expires_at: chrono::DateTime<chrono::FixedOffset>,
    pub completed_file_id: Option<Uuid>,
    pub created_by: Uuid,
}

impl From<&file_uploads::Model> for FileUploadAuditSnapshot {
    fn from(m: &file_uploads::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            file_name: m.file_name.clone(),
            expected_size: m.expected_size,
            status: m.status.clone(),
            parts_total: m.parts_total,
            parts_received: m.parts_received,
            expires_at: m.expires_at,
            completed_file_id: m.completed_file_id,
            created_by: m.created_by,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReferenceAuditSnapshot {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub file_id: Uuid,
    pub resource_type: String,
    pub resource_id: String,
    pub field_name: String,
    pub created_by: Uuid,
}

impl From<&file_references::Model> for FileReferenceAuditSnapshot {
    fn from(m: &file_references::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            file_id: m.file_id,
            resource_type: m.resource_type.clone(),
            resource_id: m.resource_id.clone(),
            field_name: m.field_name.clone(),
            created_by: m.created_by,
        }
    }
}

// ---------------------------------------------------------------------------
// Query & Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub action: Option<String>,
    pub user_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogResponse {
    pub id: String,
    pub trace_id: Option<String>,
    pub request_id: Option<String>,
    pub tenant_id: String,
    pub user_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub before_state: Option<serde_json::Value>,
    pub after_state: Option<serde_json::Value>,
    pub diff: Option<Vec<DiffEntry>>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub field: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

impl AuditLogResponse {
    pub fn from_model(m: &audit_logs::Model) -> Self {
        let diff = compute_diff(m.before_state.as_ref(), m.after_state.as_ref());

        Self {
            id: m.id.to_string(),
            trace_id: m.trace_id.clone(),
            request_id: m.request_id.clone(),
            tenant_id: m.tenant_id.to_string(),
            user_id: m.user_id.map(|u| u.to_string()),
            action: m.action.clone(),
            resource_type: m.resource_type.clone(),
            resource_id: m.resource_id.clone(),
            before_state: m.before_state.clone(),
            after_state: m.after_state.clone(),
            diff,
            ip_address: m.ip_address.clone(),
            user_agent: m.user_agent.clone(),
            status: m.status.clone(),
            error_message: m.error_message.clone(),
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

/// Compute field-level diff between before_state and after_state JSON objects.
fn compute_diff(
    before: Option<&serde_json::Value>,
    after: Option<&serde_json::Value>,
) -> Option<Vec<DiffEntry>> {
    let before_obj = before.as_ref()?.as_object()?;
    let after_obj = after.as_ref()?.as_object()?;

    let mut diffs = Vec::new();

    // Check fields in after that differ from before
    for (key, after_val) in after_obj {
        let before_val = before_obj.get(key);
        if before_val != Some(after_val) {
            diffs.push(DiffEntry {
                field: key.clone(),
                before: before_val.cloned(),
                after: Some(after_val.clone()),
            });
        }
    }

    // Check fields removed (in before but not in after)
    for (key, before_val) in before_obj {
        if !after_obj.contains_key(key) {
            diffs.push(DiffEntry {
                field: key.clone(),
                before: Some(before_val.clone()),
                after: None,
            });
        }
    }

    if diffs.is_empty() {
        None
    } else {
        Some(diffs)
    }
}
