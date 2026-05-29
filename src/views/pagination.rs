use loco_rs::prelude::model::query;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total_pages: u64,
    pub total_items: u64,
    pub page: u64,
    pub page_size: u64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn from_page_response<M>(
        pr: query::PageResponse<M>,
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
