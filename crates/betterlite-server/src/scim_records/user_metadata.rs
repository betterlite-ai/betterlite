use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserScimMetadata {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}
