use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::openai_pricing::route_predicates::is_openai_responses_route;
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::base_passthrough_logging::{CollectedChunksInput, PassthroughOutcome};
use super::super::response_payload::ResponseLoggingKwargs;
use super::tools::{PricedResponse, StandardObjectRequest};
use super::user::{payload_end_user, write_end_user};
use super::{OPENAI, OpenAiPassthroughError, OpenAiPassthroughLogger, text_or, write_call_log};

// The model LiteLLM assumes when the streamed request body names none.
const STREAM_MODEL: &str = "gpt-4o";

type Logged = fn(Json) -> PassThroughResponse;

impl OpenAiPassthroughLogger {
    // Every failure is swallowed: the entry answers with no result and empty kwargs, never the partial ones.
    pub fn openai_collected_chunks(
        &self,
        input: CollectedChunksInput<'_>,
        url_route: &str,
    ) -> Result<PassthroughOutcome, OpenAiPassthroughError> {
        match self.collected(input, url_route) {
            Ok(Some(outcome)) => Ok(outcome),
            Ok(None) => {
                tracing::warn!("Failed to build complete response from OpenAI streaming chunks");
                Ok(PassthroughOutcome {
                    result: None,
                    kwargs: ResponseLoggingKwargs::default(),
                })
            }
            Err(error) => {
                tracing::error!("Error in OpenAI streaming passthrough cost tracking: {error}");
                Ok(PassthroughOutcome {
                    result: None,
                    kwargs: ResponseLoggingKwargs::default(),
                })
            }
        }
    }

    fn collected(
        &self,
        input: CollectedChunksInput<'_>,
        url_route: &str,
    ) -> Result<Option<PassthroughOutcome>, OpenAiPassthroughError> {
        let model = text_or(
            input.request_body.get("model"),
            STREAM_MODEL,
            "request_body.model",
        )?;
        let is_responses = is_openai_responses_route(url_route);
        let completed = self.complete_response(input.all_chunks, &model, is_responses)?;
        let Some((response, logged)) = completed else {
            return Ok(None);
        };
        let provider = text_or(
            input.call_log.model_call_details.get("custom_llm_provider"),
            OPENAI,
            "model_call_details.custom_llm_provider",
        )?;
        let cost = self.tools.completion_cost(&PricedResponse {
            response: &response,
            model: &model,
            custom_llm_provider: &provider,
            call_type: is_responses.then_some(CallType::Responses),
        })?;
        let mut kwargs = stream_kwargs(&mut *input.call_log, &model, cost)?;
        write_call_log(&mut *input.call_log, &model, &provider, cost);
        let standard = StandardObjectRequest {
            response: &response,
            start_time: input.start_time,
            end_time: input.end_time,
        };
        // Stored so the success handler adopts it and keeps a Responses reply's token split.
        kwargs.standard_logging_object = self
            .tools
            .standard_logging_object(&standard, &*input.call_log);
        tracing::debug!(
            "OpenAI streaming passthrough cost tracking - Model: {model}, Cost: ${cost:.6}"
        );
        Ok(Some(PassthroughOutcome {
            result: Some(logged(response)),
            kwargs,
        }))
    }

    // A Responses route reads the terminal event; every other route rebuilds a chat response from the chunks.
    fn complete_response(
        &self,
        all_chunks: &[String],
        model: &str,
        is_responses: bool,
    ) -> Result<Option<(Json, Logged)>, OpenAiPassthroughError> {
        if is_responses {
            let responses: Logged = PassThroughResponse::ResponsesApi;
            let terminal = self.tools.terminal_response(all_chunks)?;
            return Ok(terminal.map(|response| (response, responses)));
        }
        let chat: Logged = PassThroughResponse::ModelResponse;
        let text: Logged = PassThroughResponse::TextCompletion;
        let streamed = self.tools.stream_response(all_chunks, model)?;
        Ok(streamed.map(|built| match built {
            StreamedResponse::Model(response) => (response, chat),
            StreamedResponse::TextCompletion(response) => (response, text),
        }))
    }
}

// LiteLLM's collected-stream kwargs; its call_type and messages entries have no field in B3630's kwargs.
fn stream_kwargs(
    call_log: &mut LoggingState,
    model: &str,
    cost: Decimal,
) -> Result<ResponseLoggingKwargs, OpenAiPassthroughError> {
    let mut kwargs = ResponseLoggingKwargs {
        response_cost: Some(cost),
        model: Some(model.to_owned()),
        litellm_params: call_log.litellm_params.clone(),
        standard_logging_object: None,
        passthrough_request_body: None,
    };
    // LiteLLM's one-level copy shares proxy_server_request, so the call log sees the user as well.
    if let Some(user) = payload_end_user(call_log)? {
        write_end_user(&mut kwargs.litellm_params, &user)?;
        write_end_user(&mut call_log.litellm_params, &user)?;
    }
    Ok(kwargs)
}

// The success dispatch reads the standard logging object from these entries.
pub(crate) fn passthrough_kwargs(kwargs: ResponseLoggingKwargs) -> PassThroughLoggingKwargs {
    let mut entries = IndexMap::new();
    if let Some(cost) = kwargs.response_cost {
        entries.insert("response_cost".to_owned(), decimal_json(cost));
    }
    if let Some(model) = kwargs.model {
        entries.insert("model".to_owned(), Json::Text(model));
    }
    if !kwargs.litellm_params.is_empty() {
        entries.insert(
            "litellm_params".to_owned(),
            Json::Object(kwargs.litellm_params),
        );
    }
    if let Some(object) = kwargs.standard_logging_object {
        entries.insert(STANDARD_LOGGING_OBJECT.to_owned(), object);
    }
    PassThroughLoggingKwargs { entries }
}
