use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderMergeResult {
    pub placeholder_user_id: String,
    pub merged_into_user_id: String,
    pub team_ids: Vec<String>,
}
