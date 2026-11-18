use betterlite_db::identity_rows::user::LiteLLMUserTable;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMUserTableWithKeyCount {
    #[serde(flatten)]
    pub user: LiteLLMUserTable,
    #[serde(default)]
    pub key_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListResponse {
    pub users: Vec<LiteLLMUserTableWithKeyCount>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}
