use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Deserialize)]
pub(super) struct KeyListPage {
    #[serde(default)]
    pub(super) keys: Option<Vec<ListedKey>>,
    #[serde(default)]
    pub(super) total_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListedKey {
    #[serde(default)]
    pub(super) token: Option<String>,
    #[serde(default)]
    pub(super) key_alias: Option<String>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) spend: Option<Number>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SourcePage {
    #[serde(default)]
    pub(super) keys: Option<Vec<SourceKey>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SourceKey {
    #[serde(default)]
    pub(super) models: Option<Vec<JsonValue>>,
    #[serde(default)]
    pub(super) aliases: Option<BTreeMap<String, JsonValue>>,
    #[serde(default)]
    pub(super) spend: Option<Number>,
    #[serde(default)]
    pub(super) key_alias: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) budget_id: Option<String>,
    #[serde(default)]
    pub(super) config: Option<BTreeMap<String, JsonValue>>,
    #[serde(default)]
    pub(super) created_at: Option<String>,
}

#[derive(Serialize)]
pub(super) struct KeyGenerateBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) models: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) aliases: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) spend: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) key_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) budget_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) config: Option<JsonValue>,
}

#[derive(Serialize)]
pub(super) struct KeyDeleteBody {
    pub(super) keys: Option<Vec<String>>,
    pub(super) key_aliases: Option<Vec<String>>,
}
