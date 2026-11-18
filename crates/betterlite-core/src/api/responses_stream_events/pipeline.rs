use betterlite_logging::logging_state::events::update_completion_start_time;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;

use super::record::{ResponseTarget, response_target};
use super::terminal::{self, EstimatedUsage, Terminal};
use super::{
    ProcessEventError, ResponsesEventProcessor, ResponsesEventTransform, affinity,
    decode_responses_stream_value, deltas, identity,
};
use crate::api::response_synthetic_events::estimate::{EstimateRequest, UsageTokenization};
use crate::api::response_synthetic_events::stamping::ResponsesCostCalculator;
use crate::api::responses_stream_hooks::failed::handle_logging_failed_response;
use crate::api::responses_stream_hooks::failure::handle_failure;
use crate::api::responses_stream_hooks::logging::log_completed_response;
use crate::api::responses_stream_hooks::state::ResponsesStreamHookState;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

const SSE_DONE: &str = "[DONE]";

impl<Transform, Tokenization, Calculator>
    ResponsesEventProcessor<'_, '_, Transform, Tokenization, Calculator>
where
    Transform: ResponsesEventTransform,
    Tokenization: UsageTokenization,
    Calculator: ResponsesCostCalculator + Sync,
{
    pub async fn process(
        &mut self,
        payload: &str,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        logging: &StreamLoggingObject,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, ProcessEventError<Transform::Error>> {
        if payload.is_empty() {
            return Ok(None);
        }
        if payload == SSE_DONE {
            state.finished = true;
            return Ok(None);
        }
        let start_missing = logging
            .read(&mut |state| state.completion_start_time.is_none())
            .unwrap_or(false);
        if start_missing {
            logging.edit(&mut |state| update_completion_start_time(state, self.hooks.clock.now()));
        }
        match self.handle_payload(payload, state, callback).await {
            Ok(event) => Ok(event),
            Err(error) => {
                let class = error.error_class(&state.model, state.custom_llm_provider.as_deref());
                handle_failure(self.hooks, state, callback, &class).await;
                Err(error)
            }
        }
    }

    async fn handle_payload(
        &mut self,
        payload: &str,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, ProcessEventError<Transform::Error>> {
        let Ok(decoded) = decode_responses_stream_value(payload.as_bytes(), self.limits) else {
            return Ok(None);
        };
        if !decoded.is_json_object() {
            return Ok(None);
        }
        let Some(transform) = self.transform else {
            return Err(ProcessEventError::MissingProviderConfig);
        };
        let event = transform
            .transform(&state.model, &decoded, &mut *self.details)
            .map_err(ProcessEventError::<Transform::Error>::Transform)?;
        self.dispatch_event(&decoded, event, state, callback).await
    }

    async fn dispatch_event(
        &mut self,
        parsed: &Json,
        mut event: ResponsesAPIStreamingResponse,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, ProcessEventError<Transform::Error>> {
        if matches!(parsed, Json::Object(fields) if fields.contains_key("response")) {
            let metadata = affinity::request_metadata(state);
            let provider = state.custom_llm_provider.as_deref();
            match response_target(&mut event) {
                Some(ResponseTarget::Typed(response)) => {
                    identity::rewrite_response(response, provider, &metadata)?;
                }
                Some(ResponseTarget::Value(value)) => {
                    identity::rewrite_response_value(value, provider, &metadata)?;
                }
                None => {}
            }
        }
        deltas::accumulate(&event, state);
        let provider = state.custom_llm_provider.as_deref();
        let model_id = state.hidden_params.model_id.as_deref();
        identity::encode(&mut event, provider, model_id)?;
        affinity::wrap_cipher(&mut event, state);
        if let Some(kind) = Terminal::of(&event) {
            return self
                .store_terminal(event, kind, state, callback)
                .await
                .map(Some);
        }
        Ok(Some(event))
    }

    async fn store_terminal(
        &mut self,
        mut event: ResponsesAPIStreamingResponse,
        kind: Terminal,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
    ) -> Result<ResponsesAPIStreamingResponse, ProcessEventError<Transform::Error>> {
        let failed = matches!(kind, Terminal::Failed);
        {
            let generated = format!(
                "{}{}",
                state.generated_content, state.generated_tool_arguments
            );
            let request = Json::Object(state.request_data.clone());
            let missing = Json::Null;
            let request_input = state.request_data.get("input").unwrap_or(&missing);
            let estimate = if failed {
                None
            } else {
                self.tokenization.map(|tokenization| EstimatedUsage {
                    tokenization,
                    request: EstimateRequest {
                        model: &state.model,
                        request_input,
                        responses_api_request: &request,
                        generated_text: &generated,
                    },
                })
            };
            terminal::bill(&mut event, estimate.as_ref(), self.calculator);
        }
        state.completed_response = Some(event.clone());
        if failed {
            handle_logging_failed_response(
                self.hooks,
                state,
                callback,
                &mut *self.details,
                self.calculator,
            )
            .await;
        } else {
            log_completed_response(self.hooks, state, callback, &*self.details, self.dispatch)
                .await?;
        }
        Ok(event)
    }
}
