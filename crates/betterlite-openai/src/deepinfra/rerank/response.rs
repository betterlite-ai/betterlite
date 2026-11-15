use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::Deserializer;
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseMeta, RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;

use super::RerankLimits;

pub mod error;

pub trait RerankLogging {
    fn post_call(&mut self, original_response: &str) -> Result<(), LoggingFailure>;
}

#[derive(Debug, Error)]
#[error("the rerank post-call logging hook failed")]
pub struct LoggingFailure;

#[derive(Debug, Error)]
#[error("the rerank response could not be projected")]
pub struct ResponseFailure;

#[derive(Debug, Error)]
pub enum RerankDecodeError {
    #[error("the rerank response exceeds the byte limit")]
    BodyTooLarge,
    #[error("the rerank response is not UTF-8")]
    NotUtf8,
    #[error("the rerank response is not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

pub struct ResponseRequest<'request> {
    pub model: &'request str,
    pub body: &'request [u8],
    pub logging: &'request mut dyn RerankLogging,
    pub generated: Uuid,
    pub limits: RerankLimits,
}

// a missing key, an explicit null and a value stay distinct, since LiteLLM's .get(default) tells them apart
#[derive(Debug, Deserialize)]
struct Body {
    #[serde(default)]
    scores: Presence,
    #[serde(default)]
    input_tokens: Presence,
    #[serde(default)]
    request_id: Presence,
    #[serde(default)]
    inference_status: Presence,
}

#[derive(Debug, Default)]
enum Presence {
    #[default]
    Absent,
    Null,
    Value(Json),
}

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for Presence {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Json::deserialize(deserializer)? {
            Json::Null => Self::Null,
            present @ (Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)) => Self::Value(present),
        })
    }
}

// LiteLLM converts the whole body inside one try, so any failure there returns the supplied response unchanged
#[must_use]
pub fn transform_rerank_response(
    model_response: RerankResponse,
    request: ResponseRequest<'_>,
) -> RerankResponse {
    let ResponseRequest {
        model,
        body,
        logging,
        generated,
        limits,
    } = request;
    let mut response = decoded(body, logging, limits)
        .and_then(|parsed_body| build(&parsed_body, generated))
        .unwrap_or(model_response);
    response
        .hidden_params
        .insert(String::from("model"), Json::Text(model.to_owned()));
    response
}

pub fn decode_deepinfra_rerank(
    bytes: &[u8],
    limits: RerankLimits,
) -> Result<Json, RerankDecodeError> {
    if bytes.len() > limits.max_body_bytes {
        return Err(RerankDecodeError::BodyTooLarge);
    }
    let text = from_utf8(bytes).map_err(|_source| RerankDecodeError::NotUtf8)?;
    serde_json::from_str(text).map_err(|source| RerankDecodeError::Json { source })
}

fn decoded(
    bytes: &[u8],
    logging: &mut dyn RerankLogging,
    limits: RerankLimits,
) -> Result<Body, ResponseFailure> {
    let text = from_utf8(bytes).map_err(|_source| ResponseFailure)?;
    let parsed = decode_deepinfra_rerank(bytes, limits).map_err(|_error| ResponseFailure)?;
    // LiteLLM logs as soon as the body parses as JSON, before it reads any field off the value
    logging
        .post_call(text)
        .map_err(|_failure| ResponseFailure)?;
    // LiteLLM reads every field with response_json.get, so a parsed body that is not an object fails the block
    if !matches!(parsed, Json::Object(_)) {
        return Err(ResponseFailure);
    }
    let plain = serde_json::to_value(&parsed).map_err(|_error| ResponseFailure)?;
    serde_json::from_value(plain).map_err(|_error| ResponseFailure)
}

fn build(body: &Body, generated: Uuid) -> Result<RerankResponse, ResponseFailure> {
    Ok(RerankResponse {
        id: Some(response_id(body, generated)?),
        results: Some(results(body)?),
        meta: Some(response_meta(input_tokens(body)?)),
        hidden_params: hidden_params(&inference_status(body)?),
    })
}

fn results(body: &Body) -> Result<Vec<RerankResponseResult>, ResponseFailure> {
    let values = match &body.scores {
        Presence::Absent => return Ok(Vec::new()),
        Presence::Value(Json::List(values)) => values,
        Presence::Null | Presence::Value(_) => return Err(ResponseFailure),
    };
    values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let number = finite_float(value).ok_or(ResponseFailure)?;
            // the debug text keeps the ".0" Python prints for an integral float, and an exponent stays float-looking for the oracle
            let relevance = parse_decimal_lexeme(&format!("{number:?}"))
                .map(Score)
                .map_err(|_error| ResponseFailure)?;
            let index = i64::try_from(position).map_err(|_source| ResponseFailure)?;
            Ok(RerankResponseResult {
                index,
                relevance_score: relevance,
                document: None,
            })
        })
        .collect()
}

// response_json.get("input_tokens", 0): absent is zero, an explicit null stays None
fn input_tokens(body: &Body) -> Result<Option<i64>, ResponseFailure> {
    match &body.input_tokens {
        Presence::Absent => Ok(Some(0)),
        Presence::Null => Ok(None),
        Presence::Value(Json::Number(number)) => number.as_i64().map(Some).ok_or(ResponseFailure),
        Presence::Value(_) => Err(ResponseFailure),
    }
}

// response_json.get("inference_status", {}) then .get(...) on that value, so null or a non-object fails the block
fn inference_status(body: &Body) -> Result<IndexMap<String, Json>, ResponseFailure> {
    match &body.inference_status {
        Presence::Absent => Ok(IndexMap::new()),
        Presence::Value(Json::Object(status)) => Ok(status.clone()),
        Presence::Null | Presence::Value(_) => Err(ResponseFailure),
    }
}

// request_id or str(uuid.uuid4()): a falsey value generates the UUID, and pydantic rejects any truthy non-string
fn response_id(body: &Body, generated: Uuid) -> Result<String, ResponseFailure> {
    match &body.request_id {
        Presence::Absent | Presence::Null => Ok(generated.to_string()),
        Presence::Value(Json::Text(text)) if !text.is_empty() => Ok(text.clone()),
        Presence::Value(value) if falsey(value) => Ok(generated.to_string()),
        Presence::Value(_) => Err(ResponseFailure),
    }
}

// Python truthiness over the JSON values a request id can hold: zero in either sign, "", false, [] and {}
fn falsey(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|parsed| parsed.to_bits().trailing_zeros() >= 63),
        Json::Text(text) => text.is_empty(),
        Json::List(values) => values.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}

const fn response_meta(input_tokens: Option<i64>) -> RerankResponseMeta {
    RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(RerankBilledUnits {
            search_units: None,
            total_tokens: Some(input_tokens),
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens: Some(input_tokens),
            output_tokens: Some(Some(0)),
        })),
    }
}

// Python's float() takes a JSON number and refuses everything the declared float list cannot hold
fn finite_float(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64().filter(|parsed| parsed.is_finite()),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn hidden_params(inference: &IndexMap<String, Json>) -> BTreeMap<String, Json> {
    let value = |name: &str, fallback: Json| inference.get(name).map_or(fallback, Json::clone);
    BTreeMap::from([
        (
            String::from("status"),
            value("status", Json::Text(String::from("unknown"))),
        ),
        (
            String::from("runtime_ms"),
            value("runtime_ms", Json::Number(Number::from(0))),
        ),
        (
            String::from("cost"),
            value(
                "cost",
                Json::Number(Number::from_f64(0.0).unwrap_or_else(|| Number::from(0_i32))),
            ),
        ),
        (
            String::from("tokens_generated"),
            value("tokens_generated", Json::Number(Number::from(0))),
        ),
        (
            String::from("tokens_input"),
            value("tokens_input", Json::Number(Number::from(0))),
        ),
    ])
}
