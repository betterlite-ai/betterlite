use std::collections::BTreeMap;

use betterlite_config::sections::jina_infinity_rerank::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Number;
use serde_json::error::Category;
use url::Url;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use crate::api::rerank_contracts::wire::RerankContractError;
use crate::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};
use crate::api::vector_store_records::records::score::Score;
use crate::cohere_rerank::{
    RerankParams, RerankVersion, map_rerank_params, transform_rerank_request,
};
use crate::openai_base_config::Headers;

use self::error::HostRerankError;

pub mod error;

pub const JINA_URL: &str = "https://api.jina.ai/v1/rerank";
const JINA_PATH: &str = "/v1/rerank";
const JINA_PARAMS: [&str; 4] = ["query", "top_n", "documents", "return_documents"];
const JINA_USAGE: &[&str] = &[
    "search_units",
    "total_tokens",
    "input_tokens",
    "output_tokens",
];
const INFINITY_USAGE: &[&str] = &["search_units", "total_tokens", "prompt_tokens"];
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0080_0000,
    max_event_bytes: 0x0080_0000,
    max_items: 200_000,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostProvider {
    Jina,
    Infinity,
}

#[derive(Deserialize)]
struct Wire {
    id: Option<String>,
    usage: Option<IndexMap<String, Json>>,
    results: Option<Vec<WireResult>>,
}

#[derive(Deserialize)]
struct WireResult {
    index: i64,
    relevance_score: Number,
    #[serde(default, with = "serde_with::rust::double_option")]
    document: Option<Option<Json>>,
}

// the path is replaced and the rest of the base is kept; an empty base gives the bare path and a relative one is refused
pub fn jina_complete_url(api_base: Option<&str>) -> Result<String, HostRerankError> {
    let Some(base) = api_base else {
        return Ok(JINA_URL.to_owned());
    };
    if base.is_empty() {
        return Ok(JINA_PATH.to_owned());
    }
    let mut url = Url::parse(base)
        .ok()
        .filter(|parsed| !parsed.cannot_be_a_base())
        .ok_or_else(|| HostRerankError::InvalidApiBase {
            base: base.to_owned(),
        })?;
    url.set_path(JINA_PATH);
    Ok(url.into())
}

pub fn jina_request_headers(api_key: Option<&SecretString>) -> Result<Headers, HostRerankError> {
    let key = api_key.ok_or(HostRerankError::MissingApiKey)?;
    Ok(Headers::from(
        [
            ("accept", "application/json".to_owned()),
            ("content-type", "application/json".to_owned()),
            ("authorization", format!("Bearer {}", key.expose_secret())),
        ]
        .map(|(name, value)| (name.to_owned(), value)),
    ))
}

pub fn infinity_complete_url(api_base: Option<&str>) -> Result<String, HostRerankError> {
    let trimmed = api_base
        .ok_or(HostRerankError::MissingApiBase)?
        .trim_end_matches('/');
    if trimmed.ends_with("/rerank") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/rerank"))
    }
}

// only a missing key reads the settings, a missing result is the text None, and the caller's headers overwrite every default
#[must_use]
pub fn infinity_request_headers(
    caller: &Headers,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Headers {
    let key = settings.resolve_api_key(api_key);
    let shown = key.as_ref().map_or("None", |secret| secret.expose_secret());
    let mut headers = Headers::from(
        [
            ("Authorization", format!("Bearer {shown}")),
            ("accept", "application/json".to_owned()),
            ("content-type", "application/json".to_owned()),
        ]
        .map(|(name, value)| (name.to_owned(), value)),
    );
    headers.extend(caller.clone());
    headers
}

#[must_use]
pub fn map_params(
    provider: HostProvider,
    options: &RerankParams,
    non_default_params: &RerankParams,
) -> RerankParams {
    match provider {
        HostProvider::Jina => non_default_params
            .iter()
            .filter(|(name, _)| JINA_PARAMS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        HostProvider::Infinity => map_rerank_params(RerankVersion::V1, options, non_default_params),
    }
}

pub fn transform_request(
    provider: HostProvider,
    model: &str,
    params: &RerankParams,
) -> Result<RerankParams, HostRerankError> {
    match provider {
        HostProvider::Jina => {
            let mut body = RerankParams::from([("model".to_owned(), Json::Text(model.to_owned()))]);
            body.extend(params.clone());
            Ok(body)
        }
        HostProvider::Infinity => Ok(transform_rerank_request(RerankVersion::V1, model, params)?),
    }
}

pub fn jina_rerank_cost(
    billed_units: Option<&RerankBilledUnits>,
    input_cost_per_token: Option<Decimal>,
) -> Result<(Decimal, Decimal), HostRerankError> {
    let total = billed_units.and_then(|units| units.total_tokens).flatten();
    let (Some(price), Some(tokens)) = (input_cost_per_token, total) else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let input = price
        .checked_mul(Decimal::from(tokens))
        .ok_or(HostRerankError::CostOverflow)?;
    Ok((input, Decimal::ZERO))
}

// Jina logs the raw text after its status check and before it parses; Infinity never logs
pub fn decode_host_rerank<I, P>(
    provider: HostProvider,
    status: u16,
    bytes: &[u8],
    fallback_id: I,
    post_call: P,
) -> Result<RerankResponse, HostRerankError>
where
    I: FnOnce() -> Uuid,
    P: FnOnce(&str),
{
    let text = String::from_utf8_lossy(bytes);
    if provider == HostProvider::Jina {
        if status != 200 {
            return Err(HostRerankError::Status {
                status_code: status,
                message: text.into_owned(),
            });
        }
        post_call(&text);
    }
    let wire: Wire = decode_responses_shapes(bytes, LIMITS).map_err(|error| match error {
        ShapeDecodeError::Json(source) if source.classify() != Category::Data => {
            HostRerankError::Json {
                status_code: status,
                body: text.to_string(),
                source,
            }
        }
        source @ (ShapeDecodeError::Json(_)
        | ShapeDecodeError::JsonTooLarge
        | ShapeDecodeError::TooManyItems
        | ShapeDecodeError::TooDeep) => HostRerankError::Decode { source },
    })?;
    let (meta, hidden_params) = usage_meta(provider, wire.usage.unwrap_or_default())?;
    if provider == HostProvider::Jina && wire.results.is_none() {
        return Err(HostRerankError::NoResults);
    }
    let results = wire
        .results
        .unwrap_or_default()
        .into_iter()
        .map(|entry| result(provider, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RerankResponse {
        id: Some(
            wire.id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| fallback_id().to_string()),
        ),
        results: Some(results),
        meta: Some(meta),
        hidden_params,
    })
}

fn usage_meta(
    provider: HostProvider,
    usage: IndexMap<String, Json>,
) -> Result<(RerankResponseMeta, BTreeMap<String, Json>), HostRerankError> {
    let billed_units = RerankBilledUnits {
        search_units: counter(&usage, "search_units")?,
        total_tokens: counter(&usage, "total_tokens")?,
    };
    let (tokens, consumed) = match provider {
        HostProvider::Jina => (
            RerankTokens {
                input_tokens: counter(&usage, "input_tokens")?,
                output_tokens: counter(&usage, "output_tokens")?,
            },
            JINA_USAGE,
        ),
        HostProvider::Infinity => {
            let prompt = counter(&usage, "prompt_tokens")?.flatten().unwrap_or(0);
            let total = counter(&usage, "total_tokens")?.flatten().unwrap_or(0);
            let output = total
                .checked_sub(prompt)
                .ok_or(HostRerankError::CounterOverflow)?;
            (
                RerankTokens {
                    input_tokens: Some(Some(prompt)),
                    output_tokens: Some(Some(output)),
                },
                INFINITY_USAGE,
            )
        }
    };
    let extras: IndexMap<String, Json> = usage
        .into_iter()
        .filter(|(name, _)| !consumed.contains(&name.as_str()))
        .collect();
    let mut hidden_params = BTreeMap::new();
    if !extras.is_empty() {
        hidden_params.insert("usage".to_owned(), Json::Object(extras));
    }
    let meta = RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(billed_units)),
        tokens: Some(Some(tokens)),
    };
    Ok((meta, hidden_params))
}

fn counter(
    usage: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<Option<i64>>, HostRerankError> {
    match usage.get(name) {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(None)),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(|count| Some(Some(count)))
            .ok_or(HostRerankError::Counter { name }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(HostRerankError::Counter { name })
        }
    }
}

fn result(
    provider: HostProvider,
    entry: WireResult,
) -> Result<RerankResponseResult, HostRerankError> {
    let score = parse_decimal_lexeme(&entry.relevance_score.to_string())
        .map_err(RerankContractError::ScoreOutOfRange)?;
    let document = match (provider, entry.document) {
        (_, None) | (HostProvider::Infinity, Some(None)) => None,
        (HostProvider::Jina, Some(None)) => return Err(RerankContractError::NullDocument.into()),
        (HostProvider::Infinity, Some(Some(Json::Text(text)))) if text.is_empty() => None,
        (_, Some(Some(Json::Text(text)))) => Some(RerankResponseDocument { text }),
        (HostProvider::Jina, Some(Some(Json::Object(fields)))) => {
            let Some(Json::Text(text)) = fields.get("text") else {
                return Err(HostRerankError::Document);
            };
            Some(RerankResponseDocument { text: text.clone() })
        }
        (_, Some(Some(_))) => return Err(HostRerankError::Document),
    };
    Ok(RerankResponseResult {
        index: entry.index,
        relevance_score: Score(score),
        document,
    })
}

mod tests;
