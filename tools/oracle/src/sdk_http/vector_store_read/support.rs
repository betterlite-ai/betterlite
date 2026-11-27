use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_store_http_create_read::config::OpenAiCreateReadConfig;
use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_core::api::vector_store_read::TransportRequest;
use betterlite_core::api::vector_store_read::service::{ReadConfig, VectorStoreReadConfigs};
use betterlite_core::api::vector_store_search::provider::ResolvedProvider;
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
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_provider::api::vector_store_contracts::config::Params;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::Run;

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

// The pre-call every read runs, with the fixture clock and a sink that drops the record
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

    // LiteLLM always starts the SDK read from model "vector-store" and the operation's call_type
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

// The env's OpenAI credentials, exactly as the proxy wires B010's source for the openai tag
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

impl VectorStoreReadConfigs for Configs {
    fn read_config<'source>(
        &'source self,
        resolved: &ResolvedProvider,
    ) -> Option<ReadConfig<'source>> {
        if !matches!(resolved.tag, VectorStoresConfigTag::OpenAI) {
            return None;
        }
        Some(ReadConfig(Box::new(OpenAiCreateReadConfig::new(
            &self.credentials,
            LiteLlmGlobals::default(),
            self.env_api_key.clone(),
            None,
        ))))
    }
}

pub(super) fn into_index(params: &Params) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// The base, key and kwargs one run carries, owned so the borrow the core takes lives as long as the run
pub(super) struct Transport {
    params: WriteParams,
    kwargs: IndexMap<String, Json>,
    extra_query: Option<IndexMap<String, Json>>,
    extra_body: Option<IndexMap<String, Json>>,
}

impl Transport {
    pub(super) fn new(run: &Run, api_base: &str) -> Self {
        Self {
            params: WriteParams {
                api_base: Some(api_base.to_owned()),
                api_key: run
                    .options
                    .api_key
                    .as_ref()
                    .map(|key| SecretString::from(key.clone())),
                ssl_verify: None,
            },
            kwargs: IndexMap::from([(
                String::from("litellm_call_id"),
                Json::Text(run.options.litellm_call_id.clone()),
            )]),
            extra_query: run.extra_query.as_ref().map(into_index),
            extra_body: run.extra_body.as_ref().map(into_index),
        }
    }

    pub(super) fn request<'run>(&'run self, run: &'run Run) -> TransportRequest<'run> {
        TransportRequest {
            extra_headers: run.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout: run.timeout,
            custom_llm_provider: run.custom_llm_provider.as_deref(),
            params: &self.params,
            kwargs: &self.kwargs,
        }
    }
}
