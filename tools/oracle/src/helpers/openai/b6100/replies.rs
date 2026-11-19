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
use betterlite_openai::edenai_embedding::transform_response as embedding_transform_response;
use betterlite_openai::edenai_responses::{
    ReplyRequest as ResponsesRequest, transform_response as responses_transform_response,
};
use betterlite_openai::response_events::RawResponse;
use indexmap::IndexMap;
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::fixture::Fixture;

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

pub(super) fn chat_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let headers = IndexMap::new();
    let reply = chat_transform_response(ReplyRequest {
        body: body.as_bytes(),
        headers: &headers,
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

pub(super) fn chunk_value(chunk: &Value) -> Result<Value, Box<HelperError>> {
    let parsed =
        chunk_parser(chunk, || Uuid::from_u128(1_u128), || 1_700_000_000_i64).or_value_error()?;
    let Some(Value::Object(usage)) = parsed.usage else {
        return Ok(json!({"cost": null}));
    };
    Ok(json!({"cost": usage.get("cost").and_then(Value::as_f64)}))
}

pub(super) fn embedding_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    Ok(json!({"cost": hidden_cost(
        &embedding_transform_response(body.as_bytes())
            .or_value_error()?
            .hidden_params
    )}))
}

pub(super) fn responses_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: body,
    };
    let transformed = responses_transform_response(ResponsesRequest {
        reply: &reply,
        now: 1_700_000_000_i64,
        state: &mut state,
        env: &env,
    })
    .or_value_error()?;
    let cost = match transformed.additional_headers.get(RESPONSE_COST_HEADER) {
        Some(HiddenJson::Number(number)) => number.as_f64(),
        Some(HiddenJson::Bool(_) | HiddenJson::Null | HiddenJson::Text(_))
        | Some(HiddenJson::List(_) | HiddenJson::Object(_))
        | None => None,
    };
    Ok(json!({"cost": cost}))
}

pub(super) fn messages_reply_value(body: &str) -> Result<Value, Box<HelperError>> {
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
    let cost = match state.model_call_details.get(RESPONSE_COST) {
        Some(HiddenJson::Number(number)) => number.as_f64(),
        Some(HiddenJson::Bool(_) | HiddenJson::Null | HiddenJson::Text(_))
        | Some(HiddenJson::List(_) | HiddenJson::Object(_))
        | None => None,
    };
    Ok(json!({"cost": cost}))
}
