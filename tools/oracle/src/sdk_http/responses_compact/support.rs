use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::responses_compact_call::call::CompactResponsesCall;
use betterlite_core::api::responses_compact_call::deps::{
    CompactConfigFactory, CompactResponsesDeps, ConfigRequest,
};
use betterlite_core::api::responses_compact_call::error::CompactResponsesError;
use betterlite_core::api::responses_compact_call::{acompact_responses, compact_responses};
use betterlite_core::api::responses_compact_http::service::{
    ClientSource, CompactTransport, LogContext, SharedSession,
};
use betterlite_core::api::responses_provider_selection::provider_config::{
    ConfigLookup, NativeConfig,
};
use betterlite_core::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::response_events::ResponsesReply;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::Mode;
use super::config::OpenAiCompactConfig;

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call state a production compact request logs through, with nothing attached
pub(super) struct Fixture {
    pub(super) redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new(call_type: &str) -> Result<Self, SdkHttpError> {
        let text = format!(
            r#"{{"model":"gpt-4o","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        );
        let init: LoggingInit = serde_json::from_str(&text)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(SdkHttpError::LoggingState)?;
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
        })
    }

    // Prepare then send under one logging environment, so the pre-call and the post-call share the same state
    pub(super) async fn run<S, F>(
        &mut self,
        deps: &CompactResponsesDeps<'_, S, F>,
        request: &CompactResponsesCall,
        mode: Mode,
    ) -> Result<ResponsesReply, CompactResponsesError>
    where
        S: ClientSource<Client: CompactTransport>,
        F: CompactConfigFactory,
    {
        let env = PrePostEnv {
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
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        match mode {
            Mode::Sync => compact_responses(deps, request, &mut log).await,
            Mode::Async => acompact_responses(deps, request, &mut log).await,
        }
    }
}

// The pooled client the deployment would get; the case env only carries the key and the proxy settings
pub(super) struct Outbound {
    client: ProxiedClient,
}

impl Outbound {
    pub(super) fn new(env: &BTreeMap<String, String>) -> Result<Self, SdkHttpError> {
        Ok(Self {
            client: super::super::vector_query_embedding::outbound_client(env)?,
        })
    }
}

impl ClientSource for Outbound {
    type Client = ProxiedClient;

    fn client(
        &self,
        _ssl_verify: Option<bool>,
        _shared_session: Option<SharedSession>,
    ) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

pub(super) struct Registry;

impl ConfigLookup for Registry {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        matches!(provider, "openai" | "azure" | "xai").then_some(NativeConfig {
            encrypted_agent_messages: true,
        })
    }
}

pub(super) struct ScriptedResolver;

impl ModelResolver for ScriptedResolver {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let (provider, model) = scripted(request.model, request.custom_llm_provider);
        if provider.is_empty() {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        }
        Ok(ResolvedModel {
            model,
            provider: ProviderIdentity::Custom(provider),
            api_base: None,
            api_key: None,
        })
    }
}

// get_llm_provider reads a prefix, even one the caller named; B931 strips responses/ later
pub(super) fn scripted(model: &str, custom_llm_provider: Option<&str>) -> (String, String) {
    if let Some(given) = custom_llm_provider {
        let resolved = match model.split_once('/') {
            Some((prefix, rest)) if prefix == given => rest.to_owned(),
            Some(_) | None => model.to_owned(),
        };
        return (given.to_owned(), resolved);
    }
    let Some((prefix, rest)) = model.split_once('/') else {
        return (String::new(), model.to_owned());
    };
    (prefix.to_owned(), rest.to_owned())
}

pub(super) struct Configs {
    pub(super) api_key: Option<SecretString>,
}

impl CompactConfigFactory for Configs {
    type Provider = OpenAiCompactConfig;

    fn compact_config(&self, request: &ConfigRequest<'_>) -> Option<OpenAiCompactConfig> {
        (request.provider == "openai").then(|| OpenAiCompactConfig {
            api_key: self.api_key.clone(),
        })
    }
}
