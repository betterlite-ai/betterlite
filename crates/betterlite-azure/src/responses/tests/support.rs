use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::sections::azure_responses::Settings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::reasoning_models::{
    DropParams, ReasoningCapabilities, catalogue_declares_default,
};
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_openai::response_params::params::{
    ResponsesOptions, ResponsesParams, supported_openai_params,
};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use crate::credentials::selection::AzureTokenError;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::oidc_exchange::OidcExchangeInput;
use crate::responses::capabilities::map_params;

pub(super) const MODEL_COST: &str = r#"{
  "azure/gpt-5": {
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true,
    "default_reasoning_effort": "none"
  },
  "azure/gpt-5-plain": {"supports_reasoning": true},
  "azure/gpt-5-unknown": {},
  "azure/gpt-5-nolabel": {
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true
  },
  "azure/o3": {"supports_reasoning": true},
  "o3": {"supports_reasoning": true},
  "o3-mini": {"supports_reasoning": false},
  "deepseek-r1": {"supports_reasoning": true}
}"#;

// A catalogue that declares no default effort at all, so the none-support flag answers on its own
pub(super) const NO_DEFAULT_MODEL_COST: &str = r#"{
  "azure/gpt-5-nolabel": {
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true
  }
}"#;

pub(super) struct CapabilityFixture {
    entries: BTreeMap<String, Json>,
}

fn as_bool(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(answer) => Some(*answer),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

impl CapabilityFixture {
    fn field(&self, request: &CapabilityRequest<'_>, key: &str) -> Option<&Json> {
        self.entries.get(request.model)?.as_object()?.get(key)
    }
}

impl ReasoningCapabilities for CapabilityFixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(as_bool) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(
            self.field(request, key)
                .and_then(Json::as_str)
                .map(str::to_owned),
        )
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(as_bool) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(
            self.field(request, "supports_response_schema")
                .and_then(as_bool)
                == Some(true),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self
            .entries
            .values()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect();
        catalogue_declares_default(entries.iter().map(String::as_str))
    }
}

impl ResponsesCapabilities for CapabilityFixture {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        let key = model.to_owned();
        let answer = self.entries.get(model).map(|entry| ModelInfo::Found {
            key,
            supports_reasoning: entry
                .as_object()
                .and_then(|fields| fields.get("supports_reasoning"))
                .and_then(as_bool),
        });
        ready(answer.unwrap_or(ModelInfo::Unavailable))
    }
}

pub(super) fn fixture() -> CapabilityFixture {
    CapabilityFixture {
        entries: serde_json::from_str(MODEL_COST).unwrap(),
    }
}

pub(super) fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: fixture().entries.keys().cloned().collect::<BTreeSet<_>>(),
        text_models: BTreeSet::new(),
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

pub(super) fn settings(version: &str) -> Settings {
    Settings {
        default_responses_api_version: version.to_owned(),
    }
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

pub(super) fn params(text: &str) -> ResponsesParams {
    serde_json::from_str(text).unwrap()
}

pub(super) fn options(model: &str, provider: ProviderId, text: &str) -> ResponsesOptions {
    ResponsesOptions {
        provider,
        model: model.to_owned(),
        params: params(text),
    }
}

pub(super) fn mapped(
    options: &ResponsesOptions,
    drop: DropParams,
) -> Result<serde_json::Value, ResponseParamsError> {
    mapped_with(MODEL_COST, options, drop)
}

pub(super) fn mapped_with(
    cost: &str,
    options: &ResponsesOptions,
    drop: DropParams,
) -> Result<serde_json::Value, ResponseParamsError> {
    let mut capabilities = CapabilityFixture {
        entries: serde_json::from_str(cost).unwrap(),
    };
    let mut bundled = BundledReasoningModels::default();
    runtime()
        .block_on(map_params(&mut capabilities, options, drop, &mut bundled))
        .map(|mapped| serde_json::to_value(mapped).unwrap())
}

pub(super) fn supported() -> BTreeSet<&'static str> {
    supported_openai_params()
}

pub(super) fn deployment_settings() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}

pub(super) fn environment_headers(
    headers: &[(&str, &str)],
    api_key: Option<&SecretString>,
) -> Result<BTreeMap<String, String>, AzureTokenError> {
    let sources = NoSources;
    validate_azure_environment(EnvironmentHeadersRequest {
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        api_key,
        credentials: &deployment_settings(),
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
}

pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}
