use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) function: String,
    #[serde(default)]
    pub(super) query: String,
    pub(super) top_k: Option<usize>,
    pub(super) embedding_model: Option<String>,
    pub(super) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(super) router: bool,
    pub(super) principal: Option<PrincipalFields>,
    pub(super) skill: Option<RowFields>,
    #[serde(default)]
    pub(super) skills: Vec<RowFields>,
    #[serde(default)]
    pub(super) rows: Vec<RowFields>,
    #[serde(default)]
    pub(super) vectors: BTreeMap<String, Vec<f64>>,
    pub(super) fault: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RowFields {
    pub(super) skill_id: String,
    pub(super) display_title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) instructions: Option<String>,
    pub(super) created_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrincipalFields {
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
    pub(super) org_id: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) token: Option<String>,
}
