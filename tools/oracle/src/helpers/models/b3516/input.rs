use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) template: Option<String>,
    pub(crate) timeout: Option<i64>,
    pub(crate) allow_internet_access: Option<bool>,
    pub(crate) container: Option<ContainerInput>,
    pub(crate) code: Option<String>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
    #[serde(default = "default_response_status")]
    pub(crate) response_status: u16,
    #[serde(default)]
    pub(crate) response_body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Create,
    Run,
    Delete,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ContainerInput {
    Id(String),
    Handle(ContainerFields),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainerFields {
    pub(crate) id: String,
    pub(crate) domain: Option<String>,
    pub(crate) envd_access_token: Option<String>,
    pub(crate) traffic_access_token: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
}

const fn default_response_status() -> u16 {
    200
}
