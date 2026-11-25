mod handler;
mod stream;
pub mod toolbox;
pub mod tools;
pub mod user;

pub(crate) mod tests;

use std::sync::Arc;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use thiserror::Error;

use self::tools::{ChatTransformRequest, OpenAiLoggingTools, OpenAiToolsError};
use super::base_passthrough_logging::{
    CollectedChunksInput, PassthroughChatLogger, PassthroughLoggingError, PassthroughPayload,
    PassthroughProvider, PassthroughStage, UpstreamChatReply,
};

pub(crate) const OPENAI: &str = "openai";

#[derive(Debug, Error)]
pub enum OpenAiPassthroughError {
    #[error("an OpenAI logging step failed: {source}")]
    Tools {
        #[from]
        source: OpenAiToolsError,
    },
    #[error(transparent)]
    Chat(#[from] PassthroughLoggingError),
    #[error("the kwargs carry no litellm_params to write the end user into")]
    MissingLitellmParams,
    #[error("the {field} value has the wrong type")]
    FieldType { field: &'static str },
}

impl From<OpenAiPassthroughError> for HandlerError {
    fn from(error: OpenAiPassthroughError) -> Self {
        Self::leaf(error)
    }
}

pub struct OpenAiPassthroughLogger {
    tools: Arc<dyn OpenAiLoggingTools>,
    chat: PassthroughChatLogger,
}

impl OpenAiPassthroughLogger {
    #[must_use]
    pub fn new(tools: Arc<dyn OpenAiLoggingTools>, payload: Arc<dyn PassthroughPayload>) -> Self {
        let provider = OpenAiProvider {
            tools: Arc::clone(&tools),
        };
        Self {
            chat: PassthroughChatLogger::new(Arc::new(provider), payload),
            tools,
        }
    }
}

// The base handler's OpenAI provider: the same transform and assembly, called with LiteLLM's empty arguments.
pub struct OpenAiProvider {
    tools: Arc<dyn OpenAiLoggingTools>,
}

impl PassthroughProvider for OpenAiProvider {
    fn provider_name(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn transform_chat_response(
        &self,
        model: &str,
        upstream: &UpstreamChatReply<'_>,
    ) -> Result<Json, PassthroughLoggingError> {
        let no_messages = Json::List(Vec::new());
        let no_params = Json::Object(JsonObject::new());
        let empty = JsonObject::new();
        let request = ChatTransformRequest {
            upstream,
            model,
            messages: &no_messages,
            optional_params: &no_params,
            request_data: &empty,
            json_mode: false,
            litellm_params: &empty,
        };
        self.tools
            .chat_response(&request)
            .map_err(|source| PassthroughStage::Transform.failed(source))
    }

    // ast-grep-ignore: no-long-names - B3650's fixed trait method
    fn build_complete_streaming_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughLoggingError> {
        self.tools
            .stream_response(all_chunks, model)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))
    }
}

impl CollectedChunksHandler for OpenAiPassthroughLogger {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let request_body = match call.request_body {
            None | Some(Json::Null) => JsonObject::new(),
            Some(Json::Object(body)) => body.clone(),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                tracing::error!(
                    "Error in OpenAI streaming passthrough cost tracking: the request body is not a JSON object"
                );
                return Ok(HandlerOutcome::default());
            }
        };
        let mut finished = None;
        call.logging_obj.edit(&mut |call_log| {
            let input = CollectedChunksInput {
                call_log,
                request_body: &request_body,
                start_time: call.start_time,
                end_time: call.end_time,
                all_chunks: call.lines,
            };
            finished = Some(self.openai_collected_chunks(input, call.url_route));
        });
        let Some(collected) = finished else {
            return Ok(HandlerOutcome::default());
        };
        let outcome = collected?;
        Ok(HandlerOutcome {
            result: outcome.result,
            kwargs: stream::passthrough_kwargs(outcome.kwargs),
        })
    }
}

// Text, or the default when the key is absent or null; any other type is an error naming the field.
fn text_or(
    value: Option<&Json>,
    default: &str,
    field: &'static str,
) -> Result<String, OpenAiPassthroughError> {
    match value {
        None | Some(Json::Null) => Ok(default.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(OpenAiPassthroughError::FieldType { field })
        }
    }
}

// Written before the standard object is built: B190's builder reads the call log where LiteLLM read the kwargs.
fn write_call_log(call_log: &mut LoggingState, model: &str, provider: &str, cost: Decimal) {
    model.clone_into(&mut call_log.model);
    call_log.custom_llm_provider = Some(provider.to_owned());
    let details = &mut call_log.model_call_details;
    details.insert("model".to_owned(), Json::Text(model.to_owned()));
    details.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    details.insert("response_cost".to_owned(), decimal_json(cost));
}
