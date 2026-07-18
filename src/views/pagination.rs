use loco_rs::prelude::model::query;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const fn default_page() -> u64 {
    1
}

const fn default_page_size() -> u64 {
    20
}

fn deserialize_u64_from_query<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U64QueryValue {
        Number(u64),
        String(String),
    }

    match U64QueryValue::deserialize(deserializer)? {
        U64QueryValue::Number(value) => Ok(value),
        U64QueryValue::String(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationParams {
    #[serde(
        default = "default_page",
        deserialize_with = "deserialize_u64_from_query"
    )]
    pub page: u64,
    #[serde(
        default = "default_page_size",
        deserialize_with = "deserialize_u64_from_query"
    )]
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

#[derive(Debug, Serialize, ToSchema)]
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
