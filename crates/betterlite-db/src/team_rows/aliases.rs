use super::team::LiteLLMTeamTable;
use crate::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMModelTable {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub model_aliases: Option<ModelAliases>,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub team: Option<Box<LiteLLMTeamTable>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelAliases {
    Text(String),
    Map(BTreeMap<String, JsonData>),
}
