use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::request_headers::Settings;
use betterlite_logging::end_user::extraction::IdSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_handler::types::{NO_COMPLETE_RESPONSE, StreamedResponse};
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use super::response_costing::ResponseCostService;
use super::response_payload::{
    ResponseLoggingInput, ResponseLoggingKwargs, end_user_from_request_body,
};
// ast-grep-ignore: no-long-names - B3630's fixed name, called from the shared handler
use super::response_payload::create_response_logging_payload;

pub(crate) mod tests;

// LiteLLM's httpx.Response as a provider transform reads it: the upstream status and the body bytes.
#[derive(Clone, Copy, Debug)]
pub struct UpstreamChatReply<'upstream> {
    pub status_code: u16,
    pub body: &'upstream [u8],
}

// BasePassthroughLoggingHandler's abstract members, one implementation per provider.
pub trait PassthroughProvider: Send + Sync {
    fn provider_name(&self) -> ProviderId;

    fn transform_chat_response(
        &self,
        model: &str,
        upstream: &UpstreamChatReply<'_>,
    ) -> Result<Json, PassthroughLoggingError>;

    // ast-grep-ignore: no-long-names - B3637's OpenAI provider implements this fixed name
    fn build_complete_streaming_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughLoggingError>;
}

pub struct PassthroughPayloadInput<'input> {
    pub response: &'input mut Json,
    pub model: &'input str,
    pub kwargs: ResponseLoggingKwargs,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub call_log: &'input mut LoggingState,
}

// B3630's create_response_logging_payload behind one collaborator, so tests can fake the payload step.
pub trait PassthroughPayload: Send + Sync {
    fn build(&self, input: PassthroughPayloadInput<'_>) -> ResponseLoggingKwargs;

    fn end_user(&self, request_body: Option<&JsonObject>) -> Option<String>;
}

// The production payload: B3630's steps with the settings the end-user lookup reads.
pub struct ResponsePayloadBuilder {
    cost: Arc<dyn ResponseCostService>,
    builder: Arc<dyn StandardPayloadBuilder>,
    request_headers: Settings,
    validate_end_user_id_in_db: bool,
}

impl ResponsePayloadBuilder {
    #[must_use]
    pub fn new(
        cost: Arc<dyn ResponseCostService>,
        builder: Arc<dyn StandardPayloadBuilder>,
        request_headers: Settings,
        validate_end_user_id_in_db: bool,
    ) -> Self {
        Self {
            cost,
            builder,
            request_headers,
            validate_end_user_id_in_db,
        }
    }
}

impl PassthroughPayload for ResponsePayloadBuilder {
    fn build(&self, input: PassthroughPayloadInput<'_>) -> ResponseLoggingKwargs {
        let end_user = self.end_user(input.kwargs.passthrough_request_body.as_ref());
        // ast-grep-ignore: no-long-names - B3630's fixed name, called from the shared handler
        create_response_logging_payload(
            self.cost.as_ref(),
            self.builder.as_ref(),
            ResponseLoggingInput {
                response: input.response,
                model: input.model,
                kwargs: input.kwargs,
                end_user: end_user.as_deref(),
                start_time: input.start_time,
                end_time: input.end_time,
                call_log: input.call_log,
            },
        )
    }

    fn end_user(&self, request_body: Option<&JsonObject>) -> Option<String> {
        end_user_from_request_body(
            request_body,
            &IdSettings {
                request_headers: &self.request_headers,
                validate_end_user_id_in_db: self.validate_end_user_id_in_db,
            },
        )
    }
}

// The fields passthrough_chat_handler reads from LiteLLM's arguments; url_route rides along unread for B3606.
pub struct PassthroughChatInput<'input> {
    pub upstream: UpstreamChatReply<'input>,
    pub response_body: &'input JsonObject,
    pub call_log: &'input mut LoggingState,
    pub url_route: &'input str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub request_body: &'input JsonObject,
    pub kwargs: ResponseLoggingKwargs,
}

// The fields log_collected_chunks reads; LiteLLM's url_route, endpoint type and handler are never read.
pub struct CollectedChunksInput<'input> {
    pub call_log: &'input mut LoggingState,
    pub request_body: &'input JsonObject,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub all_chunks: &'input [String],
}

#[derive(Debug)]
pub struct PassthroughOutcome {
    pub result: Option<PassThroughResponse>,
    pub kwargs: ResponseLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum PassthroughLoggingError {
    #[error("the {field} value must be a string")]
    ModelType { field: &'static str },
    #[error("the passthrough response could not be transformed: {source}")]
    Transform {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("the collected chunks could not be built into a response: {source}")]
    StreamBuild {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("the passthrough logging payload could not be built: {source}")]
    Payload {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

// The step a failure came from, so one constructor covers the three boxed variants.
#[derive(Clone, Copy, Debug)]
pub enum PassthroughStage {
    Transform,
    StreamBuild,
    Payload,
}

impl PassthroughStage {
    #[must_use]
    pub fn failed<E>(self, source: E) -> PassthroughLoggingError
    where
        E: Error + Send + Sync + 'static,
    {
        let boxed: Box<dyn Error + Send + Sync> = Box::new(source);
        match self {
            Self::Transform => PassthroughLoggingError::Transform { source: boxed },
            Self::StreamBuild => PassthroughLoggingError::StreamBuild { source: boxed },
            Self::Payload => PassthroughLoggingError::Payload { source: boxed },
        }
    }
}

pub struct PassthroughChatLogger {
    pub(crate) provider: Arc<dyn PassthroughProvider>,
    pub(crate) payload: Arc<dyn PassthroughPayload>,
}

impl PassthroughChatLogger {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B3637 and B8931 build the shared logger through this fixed constructor
    pub fn new(
        provider: Arc<dyn PassthroughProvider>,
        payload: Arc<dyn PassthroughPayload>,
    ) -> Self {
        Self { provider, payload }
    }

    pub fn passthrough_chat_handler(
        &self,
        input: PassthroughChatInput<'_>,
    ) -> Result<PassthroughOutcome, PassthroughLoggingError> {
        let model = request_model(input.request_body, Some(input.response_body))?;
        let mut response = self
            .provider
            .transform_chat_response(&model, &input.upstream)?;
        let kwargs = self.payload.build(PassthroughPayloadInput {
            response: &mut response,
            model: &model,
            kwargs: input.kwargs,
            start_time: input.start_time,
            end_time: input.end_time,
            call_log: input.call_log,
        });
        Ok(PassthroughOutcome {
            result: Some(PassThroughResponse::ModelResponse(response)),
            kwargs,
        })
    }

    pub fn log_collected_chunks(
        &self,
        input: CollectedChunksInput<'_>,
    ) -> Result<PassthroughOutcome, PassthroughLoggingError> {
        let CollectedChunksInput {
            call_log,
            request_body,
            start_time,
            end_time,
            all_chunks,
        } = input;
        let model = request_model(request_body, None)?;
        let built = self
            .provider
            .build_complete_streaming_response(all_chunks, &model)?;
        let Some(mut complete) = built else {
            tracing::error!("{NO_COMPLETE_RESPONSE}");
            return Ok(PassthroughOutcome {
                result: None,
                kwargs: ResponseLoggingKwargs::default(),
            });
        };
        let response = match &mut complete {
            StreamedResponse::Model(json) | StreamedResponse::TextCompletion(json) => json,
        };
        let kwargs = self.payload.build(PassthroughPayloadInput {
            response,
            model: &model,
            kwargs: ResponseLoggingKwargs::default(),
            start_time,
            end_time,
            call_log,
        });
        let result = match complete {
            StreamedResponse::Model(json) => PassThroughResponse::ModelResponse(json),
            StreamedResponse::TextCompletion(json) => PassThroughResponse::TextCompletion(json),
        };
        Ok(PassthroughOutcome {
            result: Some(result),
            kwargs,
        })
    }
}

// request_body["model"] else response_body["model"] else ""; a null takes the default (docs/compatibility.md).
pub(crate) fn request_model(
    request_body: &JsonObject,
    response_body: Option<&JsonObject>,
) -> Result<String, PassthroughLoggingError> {
    let (value, field) = match request_body.get("model") {
        Some(Json::Null) | None => (
            response_body.and_then(|body| body.get("model")),
            "response_body.model",
        ),
        Some(value) => (Some(value), "request_body.model"),
    };
    match value {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(model)) => Ok(model.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PassthroughLoggingError::ModelType { field })
        }
    }
}
