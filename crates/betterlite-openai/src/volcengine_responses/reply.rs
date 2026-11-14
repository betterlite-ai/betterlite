use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::responses_objects::ResponsesApiResponse;
use crate::api::responses_shapes::DeleteResponseResult;
use crate::openai_base_config::Headers;
use crate::response_events::created_at::convert_created_at;
use crate::response_events::{RawResponse, ReplyBody, ResponsesReply};
use crate::response_resources::response::{TypedReply, json_body, response_reply};
use crate::volcengine_common::VolcEngineError;

// LiteLLM wraps any reply it cannot read in VolcEngineError, carrying the reply's status and text
impl From<&RawResponse<'_>> for VolcEngineError {
    fn from(reply: &RawResponse<'_>) -> Self {
        Self {
            status_code: reply.status,
            message: reply.text.to_owned(),
            headers: Headers::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ReplyOperation {
    Get,
    Cancel,
    Delete,
    InputItems,
}

pub enum ResourceReply {
    Response(Box<TypedReply>),
    Body(Box<Json>),
}

// a reply body is a JSON object, and anything else fails as LiteLLM's constructor call does
fn parse_body(reply: &RawResponse<'_>) -> Result<IndexMap<String, Json>, VolcEngineError> {
    let parsed: Json =
        serde_json::from_str(reply.text).map_err(|_json| VolcEngineError::from(reply))?;
    match parsed {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(VolcEngineError::from(reply))
        }
    }
}

fn decode_reply(
    reply: &RawResponse<'_>,
    fields: &IndexMap<String, Json>,
) -> Result<ReplyBody, VolcEngineError> {
    let encoded = Json::Object(fields.clone());
    let value = serde_json::to_value(encoded).map_err(|_encode| VolcEngineError::from(reply))?;
    match serde_json::from_value::<ResponsesApiResponse>(value) {
        Ok(response) => Ok(ReplyBody::Typed(Box::new(response))),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the Volcengine reply did not validate; keeping its fields as sent"
            );
            Ok(ReplyBody::Lax(fields.clone()))
        }
    }
}

// LiteLLM logs the sent text, converts created_at when the reply has one, then keeps a reply that does not validate
pub fn transform_response(
    reply: &RawResponse<'_>,
    now: i64,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<ResponsesReply, VolcEngineError> {
    post_call(
        state,
        &PostCall {
            original_response: Json::Text(reply.text.to_owned()),
            input: Json::Null,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(IndexMap::new())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    );
    let mut fields = parse_body(reply)?;
    if let Some(created) = fields.get("created_at") {
        let timestamp =
            convert_created_at(created, now).map_err(|_conversion| VolcEngineError::from(reply))?;
        fields.insert(
            String::from("created_at"),
            Json::Number(Number::from(timestamp)),
        );
    }
    let response = decode_reply(reply, &fields)?;
    Ok(ResponsesReply {
        response,
        headers: reply.headers.clone(),
        additional_headers: process_response_headers(HeaderSource::Upstream(reply.headers)),
    })
}

fn delete_body(reply: &RawResponse<'_>) -> Result<Json, VolcEngineError> {
    let fields = parse_body(reply)?;
    let encoded = serde_json::to_value(Json::Object(fields.clone()))
        .map_err(|_encode| VolcEngineError::from(reply))?;
    let Ok(decoded) = serde_json::from_value::<DeleteResponseResult>(encoded) else {
        tracing::debug!("the Volcengine delete reply did not validate; keeping its fields as sent");
        return Ok(Json::Object(fields));
    };
    let validated =
        serde_json::to_value(decoded).map_err(|_encode| VolcEngineError::from(reply))?;
    serde_json::from_value(validated).map_err(|_decode| VolcEngineError::from(reply))
}

// get and cancel validate strictly, delete falls back to the fields as sent, and input items stay raw
pub fn transform_resource_reply(
    operation: ReplyOperation,
    reply: &RawResponse<'_>,
) -> Result<ResourceReply, VolcEngineError> {
    match operation {
        ReplyOperation::Get | ReplyOperation::Cancel => {
            let typed = response_reply(reply).map_err(|_record| VolcEngineError::from(reply))?;
            Ok(ResourceReply::Response(Box::new(typed)))
        }
        ReplyOperation::Delete => Ok(ResourceReply::Body(Box::new(delete_body(reply)?))),
        ReplyOperation::InputItems => match json_body(reply) {
            Ok(decoded) => Ok(ResourceReply::Body(Box::new(decoded))),
            Err(_body) => Err(VolcEngineError::from(reply)),
        },
    }
}
