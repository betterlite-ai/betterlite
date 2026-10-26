use serde::{Deserialize, Serialize};

pub const MANAGEMENT_V1_PREFIX: &str = "/management/v1";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProblemDetail {
    pub r#type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    #[serde(default)]
    pub allowed: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PageLinks {
    #[serde(rename = "self", alias = "self_link")]
    pub self_link: String,
    #[serde(default)]
    pub prev: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PageMeta {
    pub page: u64,
    pub page_size: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FacetListResponse {
    pub data: Vec<String>,
    pub meta: PageMeta,
    pub links: PageLinks,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListMeta {
    pub total_count: u64,
    pub page: u64,
    pub page_size: u64,
    pub total_pages: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListLinks {
    #[serde(rename = "self", alias = "self_link")]
    pub self_link: String,
    pub first: String,
    #[serde(default)]
    pub prev: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
    pub last: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceResponse<T> {
    pub data: T,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListResponse<T> {
    pub data: Vec<T>,
    pub meta: ListMeta,
    pub links: ListLinks,
}

mod tests;
