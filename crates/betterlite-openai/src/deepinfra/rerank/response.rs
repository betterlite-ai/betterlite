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

use super::RerankLimits;

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

#[derive(Debug, Default, Deserialize)]
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
    let projected = decoded(body, logging, limits).and_then(|body| build(body, generated));
    let mut response = match projected {
        Ok(response) => response,
        Err(_failure) => model_response,
    };
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
    let results = scores(body.scores)?;
    let input_tokens = input_tokens(body.input_tokens)?;
    let inference = inference_status(body.inference_status)?;
    let id = request_id(body.request_id, generated)?;
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

fn scores(values: Option<Option<Json>>) -> Result<Vec<RerankResponseResult>, ResponseFailure> {
    let values = match values {
        None => return Ok(Vec::new()),
        Some(Some(Json::List(values))) => values,
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
        ))
        | Some(None) => {
            return Err(ResponseFailure);
        }
    };
    let mut results = Vec::with_capacity(values.len());
    for (position, value) in values.iter().enumerate() {
        let relevance = python_float(value).and_then(score).ok_or(ResponseFailure)?;
        let index = i64::try_from(position).map_err(|_source| ResponseFailure)?;
        results.push(RerankResponseResult {
            index,
            relevance_score: relevance,
            document: None,
        });
    }
    Ok(results)
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

fn input_tokens(value: Option<Option<Json>>) -> Result<Option<i64>, ResponseFailure> {
    match value {
        None => Ok(Some(0)),
        Some(None) => Ok(None),
        Some(Some(Json::Number(number))) => number.as_i64().map(Some).ok_or(ResponseFailure),
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
        )) => Err(ResponseFailure),
    }
}

fn request_id(value: Option<Option<Json>>, generated: Uuid) -> Result<String, ResponseFailure> {
    let given = match value {
        Some(Some(Json::Text(text))) => text,
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
        ))
        | Some(None)
        | None => String::new(),
    };
    if given.is_empty() {
        return Ok(generated.to_string());
    }
    Ok(given)
}

fn inference_status(
    value: Option<Option<Json>>,
) -> Result<IndexMap<String, Json>, ResponseFailure> {
    match value {
        None => Ok(IndexMap::new()),
        Some(Some(Json::Object(status))) => Ok(status),
        Some(Some(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        ))
        | Some(None) => Err(ResponseFailure),
    }
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
