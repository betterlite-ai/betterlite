use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse, RerankResponseMeta, RerankResponseResult,
    RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;

pub type Params = IndexMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
pub type HeaderMap = BTreeMap<String, Json>;

pub const SUPPORTED_RERANK_PARAMS: [&str; 2] = ["query", "documents"];
const QUERIES: &str = "queries";
const OVERLAY_PARAMS: [&str; 5] = [
    "queries",
    "documents",
    "service_tier",
    "instruction",
    "webhook",
];
const INFERENCE_STATUS: &str = "inference_status";
const MODEL: &str = "model";
const UNKNOWN: &str = "unknown";
const REQUEST_ID: &str = "request_id";
const INPUT_TOKENS: &str = "input_tokens";
const SCORES: &str = "scores";

#[derive(Debug, Clone, Copy)]
pub struct RerankLimits {
    pub max_body_bytes: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RerankError {
    #[error("Deepinfra API Base is required; set it in the call or in DEEPINFRA_API_BASE")]
    MissingApiBase,
    #[error("Deepinfra API key is required; please set DEEPINFRA_API_KEY")]
    MissingApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

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

pub fn complete_url(api_base: Option<&str>, model: &str) -> Result<String, RerankError> {
    let Some(base) = api_base.filter(|value| !value.is_empty()) else {
        return Err(RerankError::MissingApiBase);
    };
    let cleaned = base.replace("openai", "");
    let trimmed = cleaned.trim_end_matches('/');
    Ok(format!("{trimmed}/inference/{model}"))
}

pub struct EnvironmentRequest<'request> {
    pub headers: &'request Headers,
    pub api_key: Option<&'request SecretString>,
    pub environment_key: Option<&'request SecretString>,
}

pub fn validate_environment(request: &EnvironmentRequest<'_>) -> Result<Headers, RerankError> {
    let key = request
        .api_key
        .or(request.environment_key)
        .ok_or(RerankError::MissingApiKey)?;
    let mut headers = Headers::from([
        (
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        ),
        (String::from("accept"), String::from("application/json")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    for (name, value) in request.headers {
        headers.insert(name.clone(), value.clone());
    }
    Ok(headers)
}

pub struct MapRequest<'request> {
    pub non_default_params: &'request Params,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
}

#[must_use]
pub fn map_cohere_rerank_params(request: &MapRequest<'_>) -> Params {
    let mut optional = Params::new();
    if !request.query.is_empty() {
        let repeated = Json::List(vec![
            Json::Text(request.query.to_owned());
            request.documents.len()
        ]);
        optional.insert(QUERIES.to_owned(), repeated);
    }
    for name in OVERLAY_PARAMS {
        if let Some(value) = request.non_default_params.get(name)
            && !matches!(value, Json::Null)
        {
            optional.insert(name.to_owned(), value.clone());
        }
    }
    optional
}

#[must_use]
pub fn transform_rerank_request(optional: Option<&Params>) -> Params {
    match optional {
        Some(params) => params.clone(),
        None => Params::new(),
    }
}

pub trait RerankLogging {
    fn post_call(&mut self, original_response: &str) -> Result<(), LoggingFailure>;
}

#[derive(Debug, Error)]
#[error("the rerank post-call logging hook failed")]
pub struct LoggingFailure;

#[derive(Debug, Error)]
#[error("the rerank response could not be projected")]
struct ResponseFailure;

pub struct ResponseRequest<'request> {
    pub model: &'request str,
    pub body: &'request [u8],
    pub logging: &'request mut dyn RerankLogging,
    pub generated: Uuid,
    pub limits: RerankLimits,
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
    let projected = decoded(body, logging, limits).and_then(|fields| build(&fields, generated));
    match projected {
        Ok(response) => with_model(response, model),
        Err(_failure) => with_model(model_response, model),
    }
}

fn with_model(mut response: RerankResponse, model: &str) -> RerankResponse {
    response
        .hidden_params
        .insert(MODEL.to_owned(), Json::Text(model.to_owned()));
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
    body: &[u8],
    logging: &mut dyn RerankLogging,
    limits: RerankLimits,
) -> Result<IndexMap<String, Json>, ResponseFailure> {
    let text = std::str::from_utf8(body).map_err(|_source| ResponseFailure)?;
    let parsed = decode_deepinfra_rerank(body, limits).map_err(|_error| ResponseFailure)?;
    let Json::Object(fields) = parsed else {
        return Err(ResponseFailure);
    };
    logging
        .post_call(text)
        .map_err(|_failure| ResponseFailure)?;
    Ok(fields)
}

fn build(
    fields: &IndexMap<String, Json>,
    generated: Uuid,
) -> Result<RerankResponse, ResponseFailure> {
    let results = scores(fields)?;
    let input_tokens = input_tokens(fields)?;
    let inference = inference_status(fields)?;
    let id = request_id(fields, generated)?;
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

fn scores(fields: &IndexMap<String, Json>) -> Result<Vec<RerankResponseResult>, ResponseFailure> {
    let values = match fields.get(SCORES) {
        None => return Ok(Vec::new()),
        Some(Json::List(values)) => values,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
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
    let text = format!("{value:?}");
    parse_decimal_lexeme(&text).ok().map(Score)
}

fn input_tokens(fields: &IndexMap<String, Json>) -> Result<Option<i64>, ResponseFailure> {
    match fields.get(INPUT_TOKENS) {
        None => Ok(Some(0)),
        Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number.as_i64().map(Some).ok_or(ResponseFailure),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ResponseFailure)
        }
    }
}

fn request_id(fields: &IndexMap<String, Json>, generated: Uuid) -> Result<String, ResponseFailure> {
    let given = match fields.get(REQUEST_ID) {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ResponseFailure);
        }
        None | Some(Json::Null) => String::new(),
    };
    if given.is_empty() {
        return Ok(generated.to_string());
    }
    Ok(given)
}

fn inference_status(
    fields: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, ResponseFailure> {
    match fields.get(INFERENCE_STATUS) {
        None => Ok(IndexMap::new()),
        Some(Json::Object(status)) => Ok(status.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ResponseFailure)
        }
    }
}

fn hidden_params(inference: &IndexMap<String, Json>) -> BTreeMap<String, Json> {
    let mut hidden = BTreeMap::new();
    hidden.insert(
        "status".to_owned(),
        value_or(inference, "status", Json::Text(UNKNOWN.to_owned())),
    );
    hidden.insert(
        "runtime_ms".to_owned(),
        value_or(inference, "runtime_ms", Json::Number(Number::from(0))),
    );
    hidden.insert(
        "cost".to_owned(),
        value_or(
            inference,
            "cost",
            Json::Number(Number::from_f64(0.0).unwrap_or(Number::from(0))),
        ),
    );
    hidden.insert(
        "tokens_generated".to_owned(),
        value_or(inference, "tokens_generated", Json::Number(Number::from(0))),
    );
    hidden.insert(
        "tokens_input".to_owned(),
        value_or(inference, "tokens_input", Json::Number(Number::from(0))),
    );
    hidden
}

fn value_or(inference: &IndexMap<String, Json>, name: &str, fallback: Json) -> Json {
    match inference.get(name) {
        Some(value) => value.clone(),
        None => fallback,
    }
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
    match fields.get("detail") {
        Some(Json::Object(detail)) => match detail.get("error") {
            Some(Json::Text(text)) => text.clone(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => message.to_owned(),
        },
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => {
            message.to_owned()
        }
    }
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
