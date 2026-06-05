use loco_rs::prelude::model::query;
use serde::{Deserialize, Serialize};

const fn default_page() -> u64 {
    1
}

const fn default_page_size() -> u64 {
    20
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_page_size")]
    pub page_size: u64,
}

impl From<PaginationParams> for query::PaginationQuery {
    fn from(params: PaginationParams) -> Self {
        Self {
            page: params.page,
            page_size: params.page_size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total_pages: u64,
    pub total_items: u64,
    pub page: u64,
    pub page_size: u64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn from_page_response<M>(
        pr: &query::PageResponse<M>,
        pagination: &query::PaginationQuery,
        map_fn: impl Fn(&M) -> T,
    ) -> Self {
        Self {
            items: pr.page.iter().map(map_fn).collect(),
            total_pages: pr.total_pages,
            total_items: pr.total_items,
            page: pagination.page,
            page_size: pagination.page_size,
        }
    }
}
