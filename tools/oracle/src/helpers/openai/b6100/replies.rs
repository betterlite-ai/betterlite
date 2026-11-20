use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::edenai_messages::{
    ReplyInput, transform_response as messages_transform_response,
};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use betterlite_openai::compatible_wire::{LogInput, ReplyOptions};
use betterlite_openai::edenai_chat::{
    ReplyRequest, chunk_parser, transform_response as chat_transform_response,
};
use betterlite_openai::edenai_common::RESPONSE_COST_HEADER;
use betterlite_openai::edenai_embedding::{
    EdenAiEmbeddingError, transform_response as embedding_transform_response,
};
use betterlite_openai::edenai_responses::{
    ReplyRequest as ResponsesRequest, transform_response as responses_transform_response,
};
use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::fixture::Fixture;
use super::input::Input;
use super::steps;

const ADDITIONAL_HEADERS: &str = "additional_headers";
const RESPONSE_COST: &str = "response_cost";

fn hidden_cost(hidden: &HiddenParams) -> Option<f64> {
    let Some(HiddenJson::Object(headers)) = hidden.get(ADDITIONAL_HEADERS) else {
        return None;
    };
    let Some(HiddenJson::Number(number)) = headers.get(RESPONSE_COST_HEADER) else {
        return None;
    };
    number.as_f64()
}

// the reply header map and the call log share one number reader
fn cost_number(held: Option<&HiddenJson>) -> Option<f64> {
    match held {
        Some(HiddenJson::Number(number)) => number.as_f64(),
        Some(
            HiddenJson::Bool(_)
            | HiddenJson::Null
            | HiddenJson::Text(_)
            | HiddenJson::List(_)
            | HiddenJson::Object(_),
        )
        | None => None,
    }
}

// LiteLLM raises a meaningful error object with its status, so the runner compares it as the call's failure
fn embedding_failure(error: EdenAiEmbeddingError) -> Box<HelperError> {
    let meaningful = matches!(&error, EdenAiEmbeddingError::ReplyError { .. });
    Box::new(HelperError {
        class: String::from("Exception"),
        status: meaningful.then_some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// the five replies carry their cost in their own place, so each branch reads it where that reply puts it
pub(super) fn reply_value(input: &Input) -> Result<Option<Value>, Box<HelperError>> {
    if !input.chat_replies.is_empty() {
        return Ok(Some(fixture_steps(&input.chat_replies, ReplyFamily::Chat)?));
    }
    if !input.chunks.is_empty() {
        return Ok(Some(steps(&input.chunks, chunk_cost)?));
    }
    if !input.embedding_replies.is_empty() {
        let mut values = Vec::with_capacity(input.embedding_replies.len());
        for body in &input.embedding_replies {
            let reply = embedding_transform_response(body.as_bytes()).map_err(embedding_failure)?;
            values.push(json!({"cost": hidden_cost(&reply.hidden_params)}));
        }
        return Ok(Some(json!({"steps": values})));
    }
    if !input.responses_replies.is_empty() {
        return Ok(Some(fixture_steps(
            &input.responses_replies,
            ReplyFamily::Responses,
        )?));
    }
    if !input.messages_replies.is_empty() {
        return Ok(Some(steps(&input.messages_replies, messages_cost)?));
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum ReplyFamily {
    Chat,
    Responses,
}

fn fixture_steps(bodies: &[String], family: ReplyFamily) -> Result<Value, Box<HelperError>> {
    let mut values = Vec::with_capacity(bodies.len());
    for body in bodies {
        values.push(fixture_cost(body, family)?);
    }
    Ok(json!({"steps": values}))
}

// the chat and Responses replies both log through a fixture state, and each reports its cost on the reply
fn fixture_cost(body: &str, family: ReplyFamily) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    if let ReplyFamily::Responses = family {
        let transformed = responses_transform_response(ResponsesRequest {
            reply: &RawResponse {
                status: 200,
                headers: &IndexMap::new(),
                text: body,
            },
            now: 1_700_000_000_i64,
            state: &mut state,
            env: &env,
        })
        .or_value_error()?;
        let cost = cost_number(transformed.additional_headers.get(RESPONSE_COST_HEADER));
        return Ok(json!({"cost": cost}));
    }
    let reply = chat_transform_response(ReplyRequest {
        body: body.as_bytes(),
        headers: &IndexMap::new(),
        state: &mut state,
        env: &env,
        input: LogInput {
            messages: HiddenJson::Null,
            data: RawBody::Object(IndexMap::new()),
        },
        options: ReplyOptions {
            json_mode: false,
            provider: Some("edenai"),
            base_model: None,
            identifier: || Uuid::from_u128(1_u128),
            clock: || 1_700_000_000_i64,
        },
    })
    .or_value_error()?;
    Ok(json!({"cost": hidden_cost(&reply.hidden_params)}))
}

fn chunk_cost(chunk: &Value) -> Result<Value, Box<HelperError>> {
    let parsed =
        chunk_parser(chunk, || Uuid::from_u128(1_u128), || 1_700_000_000_i64).or_value_error()?;
    let cost = match parsed.usage {
        Some(Value::Object(usage)) => usage.get("cost").and_then(Value::as_f64),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        )
        | None => None,
    };
    Ok(json!({"cost": cost}))
}

fn messages_cost(body: &String) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let limits = ApiLimits::default();
    messages_transform_response(ReplyInput {
        body: body.as_bytes(),
        status: 200,
        limits: &limits,
        state: &mut state,
    })
    .or_value_error()?;
    Ok(json!({"cost": cost_number(state.model_call_details.get(RESPONSE_COST))}))
}
