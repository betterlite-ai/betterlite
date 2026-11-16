use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) strips: Option<Vec<StripCall>>,
    pub(crate) responses: Option<Vec<ResponseCall>>,
    pub(crate) readers: Option<Vec<ReaderCall>>,
    pub(crate) catalog: Option<Vec<CatalogCall>>,
    pub(crate) token_models: Option<Vec<Option<String>>>,
    pub(crate) versions: Option<Vec<VersionCall>>,
    pub(crate) properties: Option<Vec<PropertyCall>>,
    pub(crate) validate: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StripCall {
    pub(crate) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    pub(crate) model: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReaderCall {
    pub(crate) model: Option<String>,
    pub(crate) hidden_params: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogCall {
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionCall {
    pub(crate) api_version: Option<String>,
    pub(crate) global_api_version: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PropertyCall {
    pub(crate) global_api_version: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}
