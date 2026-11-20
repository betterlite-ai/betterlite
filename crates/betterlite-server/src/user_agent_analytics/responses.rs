use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct DistinctTagResponse {
    pub(super) tag: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DistinctTagsResponse {
    pub(super) results: Vec<DistinctTagResponse>,
}

#[derive(Debug, Serialize)]
pub(super) struct TagActiveUsersResponse {
    pub(super) tag: String,
    pub(super) active_users: i64,
    pub(super) date: String,
    pub(super) period_start: Option<String>,
    pub(super) period_end: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ActiveUsersAnalyticsResponse {
    pub(super) results: Vec<TagActiveUsersResponse>,
}
