use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_core::api::vector_file_http_read::config::{OpenAiReadConfig, ReadConfig};
use betterlite_core::api::vector_file_http_read::service::ClientSource;
use betterlite_core::api::vector_file_read::service::VectorStoreFilesConfigs;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_registry::store::{
    VectorStoreCredentials, VectorStoreRegistry,
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
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::{Registry, Run};

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

// The env's OpenAI credentials, exactly as the proxy wires B010's files config for those tags
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

impl VectorStoreFilesConfigs for Configs {
    fn read_config(&self, tag: VectorStoreFilesConfigTag) -> Option<Box<dyn ReadConfig + '_>> {
        if !matches!(tag, VectorStoreFilesConfigTag::OpenAI) {
            return None;
        }
        Some(Box::new(OpenAiReadConfig::new(
            &self.credentials,
            LiteLlmGlobals::default(),
            self.env_api_key.clone(),
            None,
        )))
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call a production request runs: the same state a request would log through, with nothing attached to it
pub(super) struct Assets {
    pub(super) redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Assets {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        Ok(Self {
            redactor,
            sink: Sink,
            now,
        })
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
            Duration::default(),
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
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

// The credential accessor: the case hands the name and its values, and no other name answers
struct CaseCredentials {
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

// GenericLiteLLMParams(**kwargs) plus the base the mock answered on
pub(super) fn kwargs_of(run: &Run, api_base: &str) -> JsonObject {
    let mut kwargs = JsonObject::from([("api_base".to_owned(), Json::Text(api_base.to_owned()))]);
    if let Some(key) = &run.options.api_key {
        kwargs.insert("api_key".to_owned(), Json::Text(key.clone()));
    }
    if let Some(call_id) = &run.options.litellm_call_id {
        kwargs.insert("litellm_call_id".to_owned(), Json::Text(call_id.clone()));
    }
    if let Some(query) = &run.options.extra_query {
        kwargs.insert(
            "extra_query".to_owned(),
            Json::Object(
                query
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(after) = &run.options.after {
        kwargs.insert("after".to_owned(), Json::Text(after.clone()));
    }
    if let Some(limit) = run.options.limit {
        kwargs.insert(
            "limit".to_owned(),
            Json::Number(serde_json::Number::from(limit)),
        );
    }
    if let Some(retries) = run.options.max_retries {
        kwargs.insert(
            "max_retries".to_owned(),
            Json::Number(serde_json::Number::from(retries)),
        );
    }
    kwargs
}
