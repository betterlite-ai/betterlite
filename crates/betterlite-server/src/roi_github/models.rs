use serde::{Deserialize, Serialize};

pub const REPOSITORY_SEARCH_PAGES: i64 = 10;
pub const REPOSITORY_PAGE_ERROR: &str = "GitHub returned an unexpected repository list.";
pub const PAGINATION_ERROR: &str = "GitHub returned an unexpected pagination response.";
pub const GRAPHQL_QUERY: &str =
    "query($owner:String!, $name:String!, $number:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) { pullRequest(number:$number) {
    commits(first:100, after:$cursor) {
      totalCount pageInfo { hasNextPage endCursor }
      nodes { commit { oid message additions deletions changedFilesIfAvailable
        author { email user { login } } } }
    }
  } }
}";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubUser {
    pub login: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubHead {
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubPullListItem {
    pub number: i64,
    #[serde(default)]
    pub html_url: String,
    pub merged_at: Option<String>,
    pub updated_at: String,
    pub title: String,
    pub body: Option<String>,
    pub head: Option<GitHubHead>,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepositoryItem {
    pub full_name: String,
    pub visibility: Option<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullDetail {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: Option<GitHubUser>,
    pub merged_at: String,
    pub head: GitHubHead,
    #[serde(default)]
    pub additions: i64,
    #[serde(default)]
    pub deletions: i64,
    pub changed_files: Option<i64>,
    pub commits: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullFile {
    pub filename: Option<String>,
    pub status: Option<String>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RestAuthor {
    #[serde(default)]
    pub email: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RestCommitContent {
    #[serde(default)]
    pub message: String,
    pub author: Option<RestAuthor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RestCommit {
    #[serde(default)]
    pub sha: String,
    pub author: Option<GitHubUser>,
    #[serde(default)]
    pub commit: RestCommitContent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphQlAuthor {
    #[serde(default)]
    pub email: String,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQlCommit {
    pub oid: String,
    pub message: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files_if_available: Option<i64>,
    pub author: Option<GraphQlAuthor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphQlNode {
    pub commit: GraphQlCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQlPageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQlConnection {
    pub total_count: i64,
    pub page_info: GraphQlPageInfo,
    pub nodes: Vec<GraphQlNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQlPullRequest {
    pub commits: GraphQlConnection,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQlRepository {
    pub pull_request: Option<GraphQlPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphQlData {
    pub repository: Option<GraphQlRepository>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphQlError {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphQlResponse {
    pub data: Option<GraphQlData>,
    #[serde(default)]
    pub errors: Vec<GraphQlError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphQlVariables {
    pub owner: String,
    pub name: String,
    pub number: i64,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphQlPayload {
    pub query: String,
    pub variables: GraphQlVariables,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubUserProfile {
    pub email: Option<String>,
}
