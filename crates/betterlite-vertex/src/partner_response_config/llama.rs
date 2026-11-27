use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::chat_choices::reply::{
    ResponseReply, TransformResponseError, transform_response as transform_openai_response,
};
use betterlite_openai::chat_choices::wire::TransformedChoice;
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::containers::container_file_http::proxied::reply_headers;
use betterlite_openai::openai_base_config::forwarding::ForwardingFailure;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize as _;
use serde::Serialize;
use serde::de::value::MapDeserializer;
use uuid::Uuid;

use crate::model_policy::error::VertexAiError;
use crate::partner_openai::error::PartnerOpenAiError;
use crate::partner_openai::llama::reply::decode_reply;

use super::PartnerResponseInput;
use super::error::PartnerResponseError;

// VertexAILlama3Config.transform_response answers a full ModelResponse, read here as its dump.
#[derive(Debug, Serialize)]
struct Answer {
    id: String,
    object: &'static str,
    created: i64,
    model: String,
    choices: Vec<TransformedChoice>,
    usage: ChatUsage,
}

// The same call as the other arms without the counter: B032 runs it as its OpenAI-like response step.
pub fn llama3_transform_response<CounterFailure>(
    input: PartnerResponseInput<'_>,
) -> Result<Json, PartnerResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let PartnerResponseInput {
        model,
        json_mode,
        identifier,
        state,
        env,
        ..
    } = input;
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
    let response = transform_openai_response(
        &reply,
        state,
        env,
        log,
        move |value, _raw_headers, _hidden_headers| {
            let Some(source) = value.as_object() else {
                return Err(ForwardingFailure {
                    message: String::from("the Llama reply is not a JSON object"),
                });
            };
            let answer = llama_answer(source, model, json_mode.unwrap_or(false), identifier)
                .map_err(|error| ForwardingFailure {
                    message: error.to_string(),
                })?;
            serde_json::to_value(answer).map_err(|error| ForwardingFailure {
                message: error.to_string(),
            })
        },
    )
    .map_err(|error| vertex_error(error, input.status, &headers))?;
    Ok(serde_json::from_value(response)?)
}

// LiteLLM parses the reply with OpenAIChatCompletionResponse, so a body that is not one is a VertexAIError.
fn llama_answer(
    source: &Object,
    requested_model: &str,
    json_mode: bool,
    identifier: fn() -> Uuid,
) -> Result<Answer, PartnerOpenAiError> {
    let reply = decode_reply(source, requested_model, json_mode, identifier)?;
    let usage = reply
        .usage
        .as_ref()
        .map(usage_from)
        .transpose()?
        .unwrap_or_default();
    Ok(Answer {
        id: reply.id,
        object: "chat.completion",
        created: reply.created,
        model: reply.model,
        choices: reply.choices,
        usage,
    })
}

// Usage(**reply.get("usage", {})): a count the reply leaves out or nulls reports zero.
fn usage_from(usage: &Object) -> Result<ChatUsage, PartnerOpenAiError> {
    let mut counts = usage.clone();
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        if counts.get(name).filter(|value| !value.is_null()).is_none() {
            counts.insert(name.to_owned(), serde_json::json!(0_i32));
        }
    }
    let deserializer = MapDeserializer::<_, serde_json::Error>::new(
        counts.iter().map(|(key, value)| (key.as_str(), value)),
    );
    ChatUsage::deserialize(deserializer).map_err(|source| PartnerOpenAiError::Reply { source })
}

fn vertex_error(error: TransformResponseError, status: u16, headers: &Headers) -> VertexAiError {
    match error {
        TransformResponseError::Json(record) => VertexAiError {
            status_code: record.status_code,
            message: record.message,
            headers: record.headers,
        },
        TransformResponseError::Projection(failure) => VertexAiError {
            status_code: status,
            message: failure.message,
            headers: Some(headers.clone()),
        },
        TransformResponseError::Base { source } => VertexAiError {
            status_code: status,
            message: source.to_string(),
            headers: Some(headers.clone()),
        },
    }
}
