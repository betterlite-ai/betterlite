use std::str;

use betterlite_config::sections::jina_infinity_rerank::Settings;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{RerankBilledUnits, RerankResponse};
use crate::api::responses_shapes::wire::WireDecodeLimits;
use crate::cohere_rerank::{
    RerankParams, RerankVersion, map_rerank_params, transform_rerank_request,
};
use crate::openai_base_config::Headers;

use self::error::HostRerankError;

pub mod error;
mod wire;

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
    Ok(url.to_string())
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
    // LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
    let text = str::from_utf8(bytes).map_or_else(
        |_| format!("{} bytes that are not UTF-8", bytes.len()),
        str::to_owned,
    );
    if provider == HostProvider::Jina {
        if status != 200 {
            return Err(HostRerankError::Status {
                status_code: status,
                message: text,
            });
        }
        post_call(&text);
    }
    let answer = wire::decode_wire(status, bytes, &text)?;
    let (meta, hidden_params) = wire::usage_meta(provider, answer.usage.unwrap_or_default())?;
    if provider == HostProvider::Jina && answer.results.is_none() {
        return Err(HostRerankError::NoResults);
    }
    let results = answer
        .results
        .unwrap_or_default()
        .into_iter()
        .map(|entry| wire::result(provider, &entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RerankResponse {
        id: Some(
            answer
                .id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| fallback_id().to_string()),
        ),
        results: Some(results),
        meta: Some(meta),
        hidden_params,
    })
}

mod tests;
