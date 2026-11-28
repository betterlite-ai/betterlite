use super::super::error::SdkHttpError;
use super::config::OpenAiCreateConfig;
use async_trait::async_trait;
use betterlite_core::api::response_chat_result::records::ModelResponse;
use betterlite_core::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use betterlite_core::api::responses_compact_http::service::{ClientSource, SharedSession};
use betterlite_core::api::responses_create::bridge::{BridgeCall, ChatBridge, Created};
use betterlite_core::api::responses_create::deps::{ConfigRequest, CreateConfigFactory};
use betterlite_core::api::responses_create::error::BridgeError;
use betterlite_core::api::responses_create::interception::{
    CreateInterception, DispatchArgs, FileSearchArgs, Interception, InterceptionResult,
};
use betterlite_core::api::responses_create_http::config::Params;
use betterlite_core::api::responses_hook_policy::callbacks::{
    DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use betterlite_core::api::responses_provider_selection::provider_config::{
    ConfigLookup, NativeConfig,
};
use betterlite_core::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use serde_json::Map;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::time::Duration;
// The SDK boundary builds the caller's record, and the Python case sends the same empty ModelResponse
pub(super) fn caller_response() -> ModelResponse {
    ModelResponse {
        id: String::new(),
        created: 0,
        model: None,
        object: String::from("chat.completion"),
        system_fingerprint: None,
        choices: Vec::new(),
        usage: None,
        extra: Map::new(),
        hidden_params: Map::new(),
    }
}
pub(super) struct Sink;
impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}
pub(super) struct Fixture {
    pub(super) redactor: Redactor,
    pub(super) sink: Sink,
    pub(super) state: LoggingState,
    pub(super) now: DateTime<FixedOffset>,
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
}
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
// The caller's declared provider wins over the model prefix; a model with neither leaves the provider empty
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
impl ConfigLookup for Configs {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        (provider == "openai").then_some(NativeConfig {
            encrypted_agent_messages: true,
        })
    }
}
impl CreateConfigFactory for Configs {
    type Provider = OpenAiCreateConfig;
    fn create_config(&self, request: &ConfigRequest<'_>) -> Option<OpenAiCreateConfig> {
        (request.provider == "openai").then(|| OpenAiCreateConfig {
            api_key: self.api_key.clone(),
        })
    }
}
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HookFailure {
    pub(super) message: String,
}
pub(super) struct Nothing;
#[async_trait]
impl DeploymentHookDispatcher for Nothing {
    type Error = HookFailure;
    async fn pre_call_deployment_hook(
        &self,
        _request: Params,
        _call_type: &str,
    ) -> Result<Option<Params>, Self::Error> {
        Ok(None)
    }
}
impl NamedCallbackResolver for Nothing {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}
#[async_trait]
impl ContinuationExecutor for Nothing {
    type Error = HookFailure;
    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }
    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }
}
pub(super) struct NoInterception;
#[async_trait]
impl CreateInterception<()> for NoInterception {
    fn mcp_requested(&self, _tools: Option<&[JsonValue]>) -> bool {
        false
    }
    async fn mcp(&self, _args: &DispatchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }
    async fn file_search(&self, _args: &FileSearchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }
}
// The wrapper adds no bridge of its own; a case that reaches this one is asking for the wrong shape
pub(super) struct NoBridge;
#[async_trait]
impl ChatBridge<()> for NoBridge {
    async fn respond(
        &self,
        _call: &BridgeCall<'_>,
        _context: (),
    ) -> Result<Created<()>, BridgeError> {
        Err(BridgeError {
            message: String::from("a chat-to-responses case does not bridge"),
        })
    }
}
