use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;

use super::wire::RowTimestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillTable {
    pub skill_id: String,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub file_content: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub file_type: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub created_at: Option<RowTimestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<RowTimestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

fn default_source() -> String {
    "custom".to_owned()
}
