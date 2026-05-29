use serde::{Deserialize, Serialize};

use crate::models::_entities::notifications;

// ---- Request types ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNotificationRequest {
    pub title: String,
    pub content: String,
    /// platform | tenant_all | tenant_role
    pub notification_type: String,
    /// normal | high, defaults to normal.
    pub priority: Option<String>,
    /// Required when notification_type = tenant_role.
    pub target_role_codes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxQueryParams {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub read: Option<bool>,
}

// ---- Response types ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationResponse {
    pub id: String,
    pub title: String,
    pub content: String,
    pub notification_type: String,
    pub priority: String,
    pub status: String,
    pub target_role_codes: Option<Vec<String>>,
    pub created_at: String,
}

impl NotificationResponse {
    pub fn from_model(m: &notifications::Model) -> Self {
        Self {
            id: m.id.to_string(),
            title: m.title.clone(),
            content: m.content.clone(),
            notification_type: m.notification_type.clone(),
            priority: m.priority.clone(),
            status: m.status.clone(),
            target_role_codes: m
                .target_role_codes
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItemResponse {
    /// notification_recipients.id
    pub id: String,
    pub notification_id: String,
    pub title: String,
    pub content: String,
    pub notification_type: String,
    pub priority: String,
    pub read_at: Option<String>,
    /// Notification creation time (not recipient creation time).
    pub created_at: String,
    pub sender_name: String,
    pub sender_tenant_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountResponse {
    pub count: i64,
    pub has_forced: bool,
}
