use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::files::records::CreatePurpose;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

use super::params::ParamsWire;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateWire {
    pub(super) file: Json,
    pub(super) purpose: CreatePurpose,
    pub(super) expires_after: Option<ExpiresAfterWire>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub(super) custom_llm_provider: Option<Option<String>>,
    pub(super) extra_headers: Option<IndexMap<String, String>>,
    pub(super) extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpiresAfterWire {
    pub(super) anchor: String,
    pub(super) seconds: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteWire {
    pub(super) file_id: String,
    pub(super) model: Option<String>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub(super) custom_llm_provider: Option<Option<String>>,
    pub(super) extra_headers: Option<IndexMap<String, String>>,
    pub(super) extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetrieveWire {
    pub(super) file_id: String,
    #[serde(default, deserialize_with = "present_or_null")]
    pub(super) custom_llm_provider: Option<Option<String>>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListWire {
    pub(super) purpose: Option<String>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub(super) custom_llm_provider: Option<Option<String>>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

// a null timeout key keeps its place in the wire, so a present null is what the caller gave
fn present_or_null<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}
