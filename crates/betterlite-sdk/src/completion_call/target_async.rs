use super::error::{CompletionCallError, into_call_error};
use super::retry;
use super::target::CompletionTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::state::LoggingHandle;
use crate::call_wrapper::target::CallTarget;
use crate::completion_request::args::CompletionArgs;
use crate::completion_request::call::{FallbackCall, MockCall, PreparedCompletion};
use crate::completion_request::prepare_acompletion;
use async_trait::async_trait;
use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_core::api::chat_agentic_loop::fake_stream::FakeStreamPort;
use betterlite_models::decisions::Json;
use betterlite_openai::completion_chat_bridge::answer::chat_completion_body;
use betterlite_openai::text_completion_response::CompletionResponse;
use std::{sync::Arc, time::Duration};
mod tail;
const TEXT_COMPLETION_PROVIDERS: [&str; 3] = [
    "text-completion-openai",
    "text-completion-codestral",
    "text-completion-inception",
];
// main.py's acompletion: prepare the async call, then the tail that follows the provider answer
pub struct AsyncCompletionTarget<'ports> {
    pub inner: CompletionTarget<'ports>,
    pub callbacks: &'ports [Arc<dyn AgenticCallback>],
    pub fake_stream: &'ports dyn FakeStreamPort,
}
impl AsyncCompletionTarget<'_> {
    // acompletion's except branch sets custom_llm_provider = custom_llm_provider or "openai"
    fn async_error(
        &self,
        error: &CompletionCallError,
        model: &str,
        provider: Option<&str>,
    ) -> CallError {
        into_call_error(
            error,
            model,
            provider.unwrap_or("openai"),
            &self.inner.redactor,
        )
    }
    pub(crate) async fn sleep_mock_delay(
        &self,
        call: &MockCall,
    ) -> Result<(), CompletionCallError> {
        let Some(value) = &call.mock_delay else {
            return Ok(());
        };
        let parsed = match value {
            Json::Number(number) => number.as_f64(),
            Json::Text(text) => text.parse::<f64>().ok(),
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
        };
        let Some(seconds) = parsed else {
            return Err(CompletionCallError::Field {
                field: "mock_delay",
            });
        };
        let duration =
            Duration::try_from_secs_f64(seconds).map_err(|_error| CompletionCallError::Field {
                field: "mock_delay",
            })?;
        self.inner.sleeper.sleep(duration).await;
        Ok(())
    }
    pub(crate) fn convert_text_reply(
        &self,
        provider: Option<&str>,
        outcome: CallOutcome,
    ) -> Result<CallOutcome, CompletionCallError> {
        let Some(name) = provider else {
            return Ok(outcome);
        };
        if !TEXT_COMPLETION_PROVIDERS.contains(&name) {
            return Ok(outcome);
        }
        let CallOutcome::Response(json) = &outcome else {
            return Ok(outcome);
        };
        let Ok(response) =
            serde_json::from_value::<CompletionResponse>(serde_json::to_value(json)?)
        else {
            return Ok(outcome);
        };
        let created = self.inner.clock.now().timestamp();
        let body = chat_completion_body(&response, created, response.model.as_deref()).map_err(
            |failure| CompletionCallError::Conversion {
                message: failure.message,
            },
        )?;
        Ok(CallOutcome::Response(serde_json::from_slice(&body)?))
    }
    // the fallback loop's answer, or acompletion's no-response error
    async fn fallbacks_answer(
        &self,
        call: FallbackCall,
        model: &str,
        provider: Option<&str>,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let outcome = super::fallbacks::run_fallbacks(self, call, logging)
            .await
            .map_err(|error| self.async_error(&error, model, provider))?;
        if matches!(&outcome, CallOutcome::Response(Json::Null)) {
            return Err(self.async_error(
                &CompletionCallError::NoFallbackResponse,
                model,
                provider,
            ));
        }
        Ok(outcome)
    }
}
#[async_trait]
impl CallTarget for AsyncCompletionTarget<'_> {
    fn name(&self) -> &'static str {
        "acompletion"
    }
    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        let encoded = serde_json::to_value(&request)
            .map_err(|error| self.async_error(&CompletionCallError::from(error), "", None))?;
        let args: CompletionArgs = serde_json::from_value(encoded)
            .map_err(|error| self.async_error(&CompletionCallError::from(error), "", None))?;
        let model = args.model.clone().unwrap_or_default();
        let preparing = prepare_acompletion(&args, &self.inner.settings, &self.inner.ports);
        let prepared = Box::pin(preparing)
            .await
            .map_err(|error| self.async_error(&CompletionCallError::from(error), &model, None))?;
        let provider = provider_of(&prepared).map(str::to_owned);
        let hooked = match prepared {
            PreparedCompletion::Fallbacks(call) => {
                return self
                    .fallbacks_answer(call, &model, provider.as_deref(), logging)
                    .await;
            }
            PreparedCompletion::Mock(call) => {
                self.sleep_mock_delay(&call)
                    .await
                    .map_err(|error| self.async_error(&error, &model, provider.as_deref()))?;
                PreparedCompletion::Mock(call)
            }
            other @ PreparedCompletion::Dispatch(_)
            | other @ PreparedCompletion::ModelList(_)
            | other @ PreparedCompletion::ResponsesBridge(_)
            | other @ PreparedCompletion::McpLoop(_) => other,
        };
        let outcome = Box::pin(self.inner.run(hooked, true, logging))
            .await
            .map_err(|error| self.async_error(&error, &model, provider.as_deref()))?;
        let converted = self
            .convert_text_reply(provider.as_deref(), outcome)
            .map_err(|error| self.async_error(&error, &model, provider.as_deref()))?;
        tail::agentic(self, converted, &args, provider.as_deref())
            .await
            .map_err(|error| self.async_error(&error, &model, provider.as_deref()))
    }
    // acompletion_with_retries defaults original_function to completion, so the blocking entry runs
    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError> {
        retry::with_retries(&self.inner, request, logging, self.inner.sleeper).await
    }
}
pub(crate) fn provider_of(prepared: &PreparedCompletion) -> Option<&str> {
    match prepared {
        PreparedCompletion::Dispatch(call)
        | PreparedCompletion::ResponsesBridge(call)
        | PreparedCompletion::McpLoop(call) => call.custom_llm_provider.as_deref(),
        PreparedCompletion::Mock(call) => call.call.custom_llm_provider.as_deref(),
        PreparedCompletion::Fallbacks(_) | PreparedCompletion::ModelList(_) => None,
    }
}
