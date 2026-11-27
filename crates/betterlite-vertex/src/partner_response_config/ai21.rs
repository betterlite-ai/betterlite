use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::chat_choices::reply::{
    ResponseReply, transform_response as transform_openai_response,
};
use betterlite_openai::compatible_wire::{LogInput, ReplyOptions, decode_compatible_chat_reply};
use betterlite_openai::containers::container_file_http::proxied::reply_headers;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::openai_base_config::forwarding::ForwardingFailure;
use uuid::Uuid;

use super::PartnerResponseCall;
use super::error::PartnerResponseError;

// OpenAIGPTConfig.transform_response: the post-call log, then convert_to_model_response_object.
pub(super) fn transform_response<Counter>(
    call: PartnerResponseCall<'_, Counter>,
) -> Result<Json, PartnerResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let PartnerResponseCall { input, .. } = call;
    let headers = reply_headers(input.headers);
    let reply = ResponseReply {
        status: input.status,
        headers: &headers,
        body: input.body,
    };
    let log = LogInput {
        messages: Json::List(input.messages.to_vec()),
        data: RawBody::Object(input.request_data.clone()),
    };
    let starting_id = input.starting_id;
    let now = input.now;
    let identifier = input.identifier;
    let answer = transform_openai_response(
        &reply,
        input.state,
        input.env,
        log,
        move |value, _raw_headers, _hidden_headers| {
            let Some(source) = value.as_object() else {
                return Err(ForwardingFailure {
                    message: String::from("the Ai21 reply is not a JSON object"),
                });
            };
            let response = project(source, starting_id, now, identifier)?;
            serde_json::to_value(response).map_err(|error| ForwardingFailure {
                message: error.to_string(),
            })
        },
    )?;
    Ok(serde_json::from_value(answer)?)
}

// convert_dict_to_response: a falsy reply id keeps the caller's id, and a missing created is read from the clock.
fn project(
    source: &Object,
    starting_id: &str,
    now: i64,
    identifier: fn() -> Uuid,
) -> Result<ChatResponse, ForwardingFailure> {
    let bytes = serde_json::to_vec(source).map_err(|error| ForwardingFailure {
        message: error.to_string(),
    })?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier,
        clock: || 0,
    };
    let mut response = decode_compatible_chat_reply(&bytes, &options)
        .map_err(|error| ForwardingFailure {
            message: error.to_string(),
        })?
        .response;
    if source
        .get("id")
        .and_then(|id| id.as_str())
        .is_none_or(str::is_empty)
    {
        starting_id.clone_into(&mut response.id);
    }
    let has_created = source.get("created").is_some_and(|value| !value.is_null());
    if !has_created {
        response.created = u64::try_from(now).unwrap_or_default();
    }
    Ok(response)
}
