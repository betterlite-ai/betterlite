use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::decode::{resolved, to_object};
use super::defaults::OBJECT_PERMISSION_DEFAULTS;
use super::error::DecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Amount(
    #[serde(
        serialize_with = "rust_decimal::serde::float::serialize",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub Decimal,
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMKeyType {
    LlmApi,
    Management,
    ReadOnly,
    #[default]
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitType {
    GuaranteedThroughput,
    BestEffortThroughput,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexPermission {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AllowedVectorStoreIndexItem {
    pub index_name: String,
    pub index_permissions: Vec<IndexPermission>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InsensitiveContains {
    pub contains: String,
    pub mode: MatchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    Insensitive,
}

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    pub struct LiteLLMObjectPermissionBase {
        mcp_servers: Vec<String>,
        mcp_access_groups: Vec<String>,
        mcp_tool_permissions: BTreeMap<String, Vec<String>>,
        mcp_toolsets: Vec<String>,
        blocked_tools: Vec<String>,
        vector_stores: Vec<String>,
        agents: Vec<String>,
        agent_access_groups: Vec<String>,
        models: Vec<String>,
        search_tools: Vec<String>,
        mcp_tool_search_enabled: bool,
        skills: Vec<String>,
    }
}

impl LiteLLMObjectPermissionBase {
    pub fn resolved_json(&self) -> Result<JsonData, DecodeError> {
        let fields = to_object(self)?;
        Ok(JsonData::Object(resolved(
            &[OBJECT_PERMISSION_DEFAULTS],
            &[&fields],
        )?))
    }
}
