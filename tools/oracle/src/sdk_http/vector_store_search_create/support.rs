use std::collections::BTreeMap;
use std::future::ready;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_search_execution::config::{Params, SearchConfig};
use betterlite_core::api::vector_search_execution::openai::OpenAiSearchConfig;
use betterlite_core::api::vector_search_execution::service::signing::{
    SignFuture, SignInput, SignedSearch, SigningSeam,
};
use betterlite_core::api::vector_store_http_create_read::config::{
    CreateReadConfig, OpenAiCreateReadConfig,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_core::api::vector_store_registry::store::{
    VectorStoreCredentials, VectorStoreRegistry,
};
use betterlite_core::api::vector_store_search::service::{
    OptionalParamFilters, VectorStoreConfigs,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_transcribe::vector_store_contracts::filters::requested_create_params;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::{Registry, SearchArgs};

// The pooled client the selected deployment would get; the case env only carries the key and the proxy settings
pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

pub(super) fn source(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Source, SdkHttpError> {
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let tls = TlsPolicy::resolve(config, env, None)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Source {
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        tls,
        routing,
    })
}

// The pre-call a production request runs, and OpenAIVectorStoreConfig's inline signing, over one case
pub(super) struct Assets {
    pub(super) redactor: Redactor,
    now: DateTime<FixedOffset>,
}

impl Assets {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        Ok(Self { redactor, now })
    }

    // LiteLLM always starts the SDK call from model "vector-store" and the operation's call_type
    pub(super) fn state(&self, call_type: &str) -> Result<LoggingState, SdkHttpError> {
        let text = format!(
            r#"{{"model":"vector-store","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        );
        let init: LoggingInit = serde_json::from_str(&text)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(SdkHttpError::LoggingState)
    }

    pub(super) fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: self,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

impl LogSink for Assets {
    fn emit(&self, _record: LogRecord) {}
}

impl SigningSeam for Assets {
    fn sign(&self, config: Arc<dyn SearchConfig>, request: SignInput) -> SignFuture<'_> {
        let signed: SignedSearch = config.sign_request(request.request());
        Box::pin(ready(Ok(signed)))
    }
}

// The env's OpenAI credentials, exactly as the proxy wires B010's config for those tags
pub(super) struct Configs {
    credentials: OpenAiCredentialsSettings,
    env_api_key: Option<SecretString>,
}

impl Configs {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            credentials: config.sections.openai_credentials.clone(),
            env_api_key: config.sections.resource_sdk_settings.openai_api_key.clone(),
        }
    }
}

impl VectorStoreConfigs for Configs {
    fn create_config(&self, tag: VectorStoresConfigTag) -> Option<Box<dyn CreateReadConfig + '_>> {
        if !matches!(tag, VectorStoresConfigTag::OpenAI) {
            return None;
        }
        Some(Box::new(OpenAiCreateReadConfig::new(
            &self.credentials,
            LiteLlmGlobals::default(),
            self.env_api_key.clone(),
            None,
        )))
    }

    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>> {
        if !matches!(tag, VectorStoresConfigTag::OpenAI) {
            return None;
        }
        Some(Arc::new(self.search_mapper()))
    }
}

impl Configs {
    fn search_mapper(&self) -> OpenAiSearchConfig {
        OpenAiSearchConfig::new(
            self.credentials.clone(),
            LiteLlmGlobals::default(),
            self.env_api_key.clone(),
            None,
        )
    }
}

// B1058 over the key set the case sends: the declared keys whose values are not null, and no mapper keys
pub(super) struct Filters;

impl OptionalParamFilters for Filters {
    fn create_params(&self, params: &Params) -> Params {
        requested_create_params(params)
    }

    fn search_params(&self, params: &Params, _tag: VectorStoresConfigTag) -> Params {
        let declared = [
            "filters",
            "max_num_results",
            "ranking_options",
            "rewrite_query",
        ];
        params
            .iter()
            .filter(|(name, value)| {
                declared.contains(&name.as_str()) && !matches!(value, Json::Null)
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

pub(crate) mod executors;

pub(super) use executors::Executors;

// The credential accessor: the case hands the name and its values, and no other name answers
pub(super) struct CaseCredentials {
    name: String,
    values: JsonObject,
}

impl VectorStoreCredentials for CaseCredentials {
    fn credential_values(&self, credential_name: &str) -> JsonObject {
        if credential_name == self.name {
            return self.values.clone();
        }
        JsonObject::new()
    }
}

pub(super) fn registry_of(registry: &Registry) -> VectorStoreRegistry {
    let mut built = VectorStoreRegistry::default();
    built.stores.push(LiteLlmManagedVectorStore {
        vector_store_id: Some(registry.vector_store_id.clone()),
        custom_llm_provider: None,
        vector_store_name: None,
        vector_store_description: None,
        vector_store_metadata: None,
        created_at: None,
        updated_at: None,
        litellm_credential_name: Some(registry.credential_name.clone()),
        litellm_params: None,
        team_id: None,
        user_id: None,
        is_config: None,
        extra: BTreeMap::new(),
    });
    built.credentials = Arc::new(CaseCredentials {
        name: registry.credential_name.clone(),
        values: registry
            .credential_values
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    });
    built
}

// The four named search fields as Params, with the two required ones kept out of the optional map
pub(super) fn search_optionals(args: &SearchArgs) -> Params {
    let mut optional = Params::new();
    if let Some(filters) = &args.filters {
        optional.insert("filters".to_owned(), Json::Object(into_index(filters)));
    }
    if let Some(limit) = args.max_num_results {
        optional.insert(
            "max_num_results".to_owned(),
            Json::Number(serde_json::Number::from(limit)),
        );
    }
    if let Some(ranking) = &args.ranking_options {
        optional.insert(
            "ranking_options".to_owned(),
            Json::Object(into_index(ranking)),
        );
    }
    if let Some(rewrite) = args.rewrite_query {
        optional.insert("rewrite_query".to_owned(), Json::Bool(rewrite));
    }
    optional
}

pub(super) fn into_index(params: &Params) -> indexmap::IndexMap<String, Json> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
