use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseMeta, RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;

use super::{HeaderMap, RerankLimits};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

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

// missing keys stay None, explicit nulls become Some(None), and any value becomes Some(Some(value))
#[derive(Debug, Deserialize)]
struct Body {
    #[serde(default, with = "serde_with::rust::double_option")]
    scores: Option<Option<Json>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    input_tokens: Option<Option<Json>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    request_id: Option<Option<Json>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    inference_status: Option<Option<Json>>,
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
        .and_then(|body| build(body, generated))
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
    let text = std::str::from_utf8(bytes).map_err(|_source| RerankDecodeError::NotUtf8)?;
    serde_json::from_str(text).map_err(|source| RerankDecodeError::Json { source })
}

fn decoded(
    bytes: &[u8],
    logging: &mut dyn RerankLogging,
    limits: RerankLimits,
) -> Result<Body, ResponseFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_source| ResponseFailure)?;
    let parsed = decode_deepinfra_rerank(bytes, limits).map_err(|_error| ResponseFailure)?;
    let body = serde_json::from_value(parsed).map_err(|_error| ResponseFailure)?;
    logging
        .post_call(text)
        .map_err(|_failure| ResponseFailure)?;
    Ok(body)
}

fn build(body: Body, generated: Uuid) -> Result<RerankResponse, ResponseFailure> {
    let results = match body.scores {
        None => Vec::new(),
        Some(Some(Json::List(values))) => values
            .iter()
            .enumerate()
            .map(|(position, value)| {
                let relevance = python_float(value).and_then(score).ok_or(ResponseFailure)?;
                let index = i64::try_from(position).map_err(|_source| ResponseFailure)?;
                Ok(RerankResponseResult {
                    index,
                    relevance_score: relevance,
                    document: None,
                })
            })
            .collect::<Result<Vec<_>, ResponseFailure>>()?,
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
        ))
        | Some(None) => return Err(ResponseFailure),
    };
    let input_tokens = match body.input_tokens {
        None => Some(0),
        Some(None) => None,
        Some(Some(Json::Number(number))) => Some(number.as_i64().ok_or(ResponseFailure)?),
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
        )) => return Err(ResponseFailure),
    };
    let inference = match body.inference_status {
        None => IndexMap::new(),
        Some(Some(Json::Object(status))) => status,
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        ))
        | Some(None) => return Err(ResponseFailure),
    };
    let given = match body.request_id {
        Some(Some(Json::Text(text))) => text,
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
        ))
        | Some(None)
        | None => String::new(),
    };
    let id = if given.is_empty() {
        generated.to_string()
    } else {
        given
    };
    let meta = RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(RerankBilledUnits {
            search_units: None,
            total_tokens: Some(input_tokens),
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens: Some(input_tokens),
            output_tokens: Some(Some(0)),
        })),
    };
    Ok(RerankResponse {
        id: Some(id),
        results: Some(results),
        meta: Some(meta),
        hidden_params: hidden_params(&inference),
    })
}

// Python's float() takes a JSON number and refuses everything the declared float list cannot hold
fn python_float(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64().filter(|parsed| parsed.is_finite()),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// the debug text keeps the ".0" Python prints for an integral float, and an exponent stays float-looking for the oracle
fn score(value: f64) -> Option<Score> {
    parse_decimal_lexeme(&format!("{value:?}")).ok().map(Score)
}

fn hidden_params(inference: &IndexMap<String, Json>) -> BTreeMap<String, Json> {
    let value = |name: &str, fallback: Json| match inference.get(name) {
        Some(value) => value.clone(),
        None => fallback,
    };
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
                Json::Number(Number::from_f64(0.0).unwrap_or(Number::from(0))),
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

// LiteLLM picks {"detail": {"error": ...}} first, then a string detail, and swallows every parse or type failure
#[must_use]
pub fn decode_deepinfra_error(message: &str, limits: RerankLimits) -> String {
    if message.len() > limits.max_body_bytes {
        return message.to_owned();
    }
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(message) else {
        return message.to_owned();
    };
    let selected = match fields.get("detail") {
        Some(Json::Object(detail)) => match detail.get("error") {
            Some(Json::Text(text)) => Some(text.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => None,
    };
    selected.unwrap_or_else(|| message.to_owned())
}

#[must_use]
pub fn get_error_class(
    error_message: &str,
    status_code: u16,
    headers: HeaderMap,
    limits: RerankLimits,
) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: decode_deepinfra_error(error_message, limits),
        headers,
    }
}
