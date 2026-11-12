mod reports;
mod sources;
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::helpers::HelperResult;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialParams {
    #[serde(rename = "api_base")]
    base: Option<String>,
    #[serde(rename = "api_key")]
    key: Option<String>,
    #[serde(rename = "api_version")]
    version: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    api_base: Option<String>,
    api_key: Option<String>,
    azure_key: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialStep {
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    route: String,
    #[serde(default)]
    default_api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentStep {
    #[serde(default)]
    azure_endpoint: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentStep {
    #[serde(default)]
    params: BTreeMap<String, Option<String>>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderStep {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    token_params: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageStep {
    api_base: String,
    #[serde(default)]
    api_version: Option<String>,
    route: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    CredentialPrecedence(Vec<CredentialStep>),
    DeploymentBaseUrl(Vec<DeploymentStep>),
    V1LatestPreview(Vec<UrlStep>),
    QueryApiVersionWins(Vec<UrlStep>),
    AppendRouteOnce(Vec<UrlStep>),
    AzureHeaderSelection(Vec<HeaderStep>),
    AzureEnvNoneVsEmpty(Vec<EnvironmentStep>),
    V1ImageResourceReset(Vec<ImageStep>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reports::invoke(input)
    }
}
