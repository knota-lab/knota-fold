use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedMenuTreeResponse {
    pub id: String,
    pub parent_id: Option<String>,
    pub code: String,
    pub name: String,
    pub path: Option<String>,
    pub alias: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: String,
    pub is_cache: bool,
    pub sort_order: i32,
    pub children: Vec<Self>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOverrideRequest {
    pub custom_name: Option<String>,
    pub custom_icon: Option<String>,
    pub custom_sort: Option<i32>,
    pub is_hidden: Option<bool>,
}
