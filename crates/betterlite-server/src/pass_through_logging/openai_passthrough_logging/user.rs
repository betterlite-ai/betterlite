use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_handler::types::PASSTHROUGH_LOGGING_PAYLOAD;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::super::base_passthrough_logging::{PassthroughPayload, PassthroughPayloadInput};
use super::super::response_costing::ResponseCostService;
// ast-grep-ignore: no-long-names - B3630's fixed name, called with OpenAI's end user
use super::super::response_payload::create_response_logging_payload;
use super::super::response_payload::{ResponseLoggingInput, ResponseLoggingKwargs};
use super::OpenAiPassthroughError;

// OpenAI's _get_user_from_metadata: request_body["user"] as is, never the end-user lookup of the base handler.
#[must_use]
pub fn openai_end_user(request_body: Option<&JsonObject>) -> Option<String> {
    let Some(Json::Text(user)) = request_body?.get("user") else {
        return None;
    };
    (!user.is_empty()).then(|| user.clone())
}

// The streaming entry finds the payload in the call log, where the pass-through setup stored it.
pub(crate) fn payload_end_user(
    call_log: &LoggingState,
) -> Result<Option<String>, OpenAiPassthroughError> {
    let payload = match call_log.model_call_details.get(PASSTHROUGH_LOGGING_PAYLOAD) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(payload)) => payload,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(OpenAiPassthroughError::FieldType {
                field: "passthrough_logging_payload",
            });
        }
    };
    match payload.get("request_body") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(request_body)) => Ok(openai_end_user(Some(request_body))),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(OpenAiPassthroughError::FieldType {
                field: "passthrough_logging_payload.request_body",
            })
        }
    }
}

// LiteLLM's setdefault chain: proxy_server_request and its body keep every key they already hold.
pub(crate) fn write_end_user(
    litellm_params: &mut JsonObject,
    user: &str,
) -> Result<(), OpenAiPassthroughError> {
    let request = object_entry(
        litellm_params,
        "proxy_server_request",
        "litellm_params.proxy_server_request",
    )?;
    let body = object_entry(request, "body", "litellm_params.proxy_server_request.body")?;
    body.insert("user".to_owned(), Json::Text(user.to_owned()));
    Ok(())
}

fn object_entry<'map>(
    map: &'map mut JsonObject,
    key: &str,
    field: &'static str,
) -> Result<&'map mut JsonObject, OpenAiPassthroughError> {
    let slot = map.entry(key.to_owned()).or_insert(Json::Null);
    // A null entry is the absent one (docs/compatibility.md), so it gets a fresh object.
    if matches!(slot, Json::Null) {
        *slot = Json::Object(JsonObject::new());
    }
    match slot {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(OpenAiPassthroughError::FieldType { field })
        }
    }
}

// The fallback's payload step: B3630's steps with OpenAI's end user in place of the base lookup.
pub struct OpenAiPayload {
    pub cost: Arc<dyn ResponseCostService>,
    pub builder: Arc<dyn StandardPayloadBuilder>,
}

impl PassthroughPayload for OpenAiPayload {
    fn build(&self, input: PassthroughPayloadInput<'_>) -> ResponseLoggingKwargs {
        let end_user = openai_end_user(input.kwargs.passthrough_request_body.as_ref());
        // ast-grep-ignore: no-long-names - B3630's fixed name, called with OpenAI's end user
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
        openai_end_user(request_body)
    }
}
