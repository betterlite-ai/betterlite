use std::collections::BTreeMap;

use betterlite_error::facts::presence::Field;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::CreatePurpose;
use indexmap::IndexMap;
use serde::Deserialize;

use super::params::ParamsWire;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateWire {
    pub(super) file: Json,
    pub(super) purpose: CreatePurpose,
    pub(super) expires_after: Option<ExpiresAfterWire>,
    #[serde(default)]
    pub(super) custom_llm_provider: Field<String>,
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
    #[serde(default)]
    pub(super) custom_llm_provider: Field<String>,
    pub(super) extra_headers: Option<IndexMap<String, String>>,
    pub(super) extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetrieveWire {
    pub(super) file_id: String,
    #[serde(default)]
    pub(super) custom_llm_provider: Field<String>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListWire {
    pub(super) purpose: Option<String>,
    #[serde(default)]
    pub(super) custom_llm_provider: Field<String>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) params: ParamsWire,
}
