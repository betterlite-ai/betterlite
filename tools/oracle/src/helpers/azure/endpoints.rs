mod fixture;
mod reports;
mod steps;
mod tests;

use std::collections::BTreeMap;

use betterlite_azure::endpoints::cloudflare::CloudflareParams;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use secrecy::SecretString;
use serde::Deserialize;

use crate::helpers::HelperResult;

pub(crate) use fixture::{Script, SourceScripts};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Defaults {
    pub(crate) default_max_retries: i64,
    pub(crate) azure_default_api_version: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientStep {
    #[serde(default)]
    pub(crate) defaults: Option<Defaults>,
    #[serde(default)]
    pub(crate) params: LiteLlmParams,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) is_async: bool,
    #[serde(default)]
    pub(crate) session: SessionChoice,
    #[serde(default)]
    pub(crate) refresh: bool,
    #[serde(default)]
    pub(crate) provider: Option<ProviderScript>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) sources: SourceScripts,
    #[serde(default)]
    pub(crate) cached: CachedChoice,
    #[serde(default)]
    pub(crate) supplied: Option<SuppliedStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloudflareStep {
    pub(crate) api_base: String,
    pub(crate) model: String,
    pub(crate) api_version: String,
    pub(crate) max_retries: i64,
    pub(crate) timeout: f64,
    #[serde(default)]
    pub(crate) params: CloudflareParams,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) azure_ad_token: Option<SecretString>,
    #[serde(default)]
    pub(crate) provider: Option<ProviderScript>,
    #[serde(default)]
    pub(crate) shared_session: bool,
    #[serde(default)]
    pub(crate) is_async: bool,
    #[serde(default)]
    pub(crate) client: Option<SuppliedStep>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) sources: SourceScripts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DynamicStep {
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) custom_query: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuppliedStep {
    pub(crate) kind: SuppliedKind,
    #[serde(default)]
    pub(crate) custom_query: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) enum SuppliedKind {
    #[serde(rename = "azure_open_ai")]
    Azure,
    #[serde(rename = "async_azure_open_ai")]
    AsyncAzure,
    #[serde(rename = "open_ai")]
    Standard,
    #[serde(rename = "async_open_ai")]
    AsyncStandard,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderScript {
    pub(crate) identity: u64,
    pub(crate) script: Script,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CachedChoice {
    #[default]
    None,
    AzureOpenAi,
    AsyncAzureOpenAi,
    OpenAi,
    AsyncOpenAi,
    Other,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionChoice {
    #[default]
    Dedicated,
    Shared,
}

impl SessionChoice {
    pub(crate) const fn session(self) -> SessionKind {
        match self {
            Self::Dedicated => SessionKind::Dedicated,
            Self::Shared => SessionKind::Shared,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    DefaultApiVersion(Vec<ClientStep>),
    CloudflareBaseModel(Vec<CloudflareStep>),
    SeparateTokenClientIdentity(Vec<ClientStep>),
    AzureSdkClientMode(Vec<ClientStep>),
    AzureInitializerCredentials(Vec<ClientStep>),
    AzureV1ClientProjection(Vec<ClientStep>),
    DynamicApiVersion(Vec<DynamicStep>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
