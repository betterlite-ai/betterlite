use async_trait::async_trait;
use betterlite_core::api::chat_agentic_loop::fake_stream::FakeStreamPort;
use betterlite_core::api::chat_agentic_loop::{
    ChatLoopResult, LoggingHandle as AgenticLoggingHandle,
};
use betterlite_models::decisions::Json;
use betterlite_models::request_pricing::register::CustomPricingRequest;
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use betterlite_sdk::completion_call::ports::{LocalTokenCountRequest, LocalTokenCounter, Sleeper};
use betterlite_sdk::completion_request::args::JsonObject;
use betterlite_sdk::completion_request::call::PreparedTimeout;
use betterlite_sdk::completion_request::error::{
    CompletionRequestError, PricingFailure, RequestResult,
};
use betterlite_sdk::completion_request::ports::{
    CacheSeedRequest, CompletionLogging, CompletionTimeout, EnvironmentUpdate, PricingRegistration,
    PromptManagement, PromptManagementError, PromptOutcome, PromptRequest, TimeoutRequest,
};
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use chrono::{DateTime, FixedOffset, Utc};
use std::time::{Duration, Instant};
// LiteLLM reads the clock out of the wrapper: utcnow for the response, monotonic for elapsed time
pub(crate) struct SystemClock {
    started: Instant,
}
impl SystemClock {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}
impl WrapperClock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
    fn monotonic(&self) -> Duration {
        self.started.elapsed()
    }
}
pub(crate) struct SystemIds;
impl IdSource for SystemIds {
    fn next_call_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
pub(crate) struct TokioSleeper;
#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}
// the SDK hands a response through unchanged when no fake stream is in play
pub(crate) struct IdentityStream;
impl FakeStreamPort for IdentityStream {
    fn wrap_model_response(
        &self,
        response: Box<serde_json::value::RawValue>,
        _model: &str,
        _provider: &str,
        _logging: &AgenticLoggingHandle<'_>,
    ) -> ChatLoopResult {
        ChatLoopResult::Response(response)
    }
}
// prompt management, the proxy's custom pricing and the logging callbacks belong to other tasks
pub(crate) struct NoPrompt;
#[async_trait]
impl PromptManagement for NoPrompt {
    fn seed_cache_control(
        &self,
        _request: &CacheSeedRequest<'_>,
        _non_default_params: &mut JsonObject,
    ) {
    }
    fn should_run(
        &self,
        _prompt_id: Option<&str>,
        _non_default_params: &JsonObject,
        _tools: Option<&[Json]>,
    ) -> bool {
        false
    }
    async fn get_chat_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        Ok(prompt_outcome(request))
    }
    async fn async_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        Ok(prompt_outcome(request))
    }
}
fn prompt_outcome(request: PromptRequest) -> PromptOutcome {
    PromptOutcome {
        model: request.model,
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}
pub(crate) struct NoPricing;
#[async_trait]
impl PricingRegistration for NoPricing {
    async fn register_custom_pricing(
        &self,
        _request: &CustomPricingRequest<'_>,
    ) -> Result<(), PricingFailure> {
        Ok(())
    }
}
pub(crate) struct NoLogging;
impl CompletionLogging for NoLogging {
    fn update_environment(&self, _update: &EnvironmentUpdate<'_>) {}
}
const COMPLETION_FALLBACK: f64 = 600.0;
fn number_of(value: &Json, field: &'static str) -> RequestResult<f64> {
    let parsed = match value {
        Json::Number(number) => number.as_f64(),
        Json::Text(text) => text.parse::<f64>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    };
    let seconds = parsed.ok_or(CompletionRequestError::FieldType {
        field,
        expected: "a number of seconds",
    })?;
    if !seconds.is_finite() || seconds < 0.0_f64 {
        return Err(CompletionRequestError::FieldType {
            field,
            expected: "a non-negative number of seconds",
        });
    }
    Ok(seconds)
}
// the shared transport enforces one deadline, so an httpx-style mapping collapses to its read phase
fn seconds_of(value: Option<&Json>, field: &'static str) -> RequestResult<Option<f64>> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(phases)) => match phases.get("read") {
            Some(read) => number_of(read, field).map(Some),
            None => Ok(Some(COMPLETION_FALLBACK)),
        },
        Some(given) => number_of(given, field).map(Some),
    }
}
// the model's timeout, then the timeout kwarg, then request_timeout, then the configured global or 600 s
pub(crate) struct SessionTimeouts;
impl CompletionTimeout for SessionTimeouts {
    fn resolve(&self, request: &TimeoutRequest<'_>) -> RequestResult<PreparedTimeout> {
        let configured = request
            .configured_request_timeout
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0_f64);
        Ok(PreparedTimeout::Seconds(
            seconds_of(request.model_timeout, "timeout")?
                .or(seconds_of(request.timeout, "timeout")?)
                .or(seconds_of(request.request_timeout, "request_timeout")?)
                .or(configured)
                .unwrap_or(COMPLETION_FALLBACK),
        ))
    }
}
// LiteLLM's local counting: the tokenizer picks the model's encoding and counts the messages
pub(crate) struct LocalCounter<'counter> {
    pub(crate) counter: &'counter Counter,
}
#[async_trait]
impl LocalTokenCounter for LocalCounter<'_> {
    async fn count(&self, request: &LocalTokenCountRequest<'_>) -> usize {
        let mut messages = Vec::with_capacity(request.messages.len());
        for message in request.messages {
            let Ok(encoded) = serde_json::to_value(message) else {
                continue;
            };
            let Ok(parsed) = serde_json::from_value::<Message>(encoded) else {
                continue;
            };
            messages.push(parsed);
        }
        let counting = TokenRequest {
            model: request.model,
            custom_tokenizer: None,
            text: None,
            messages: Some(&messages),
            count_response_tokens: None,
            tools: request.tools,
            tool_choice: None,
        };
        self.counter.count(&counting).await.unwrap_or(0)
    }
}
