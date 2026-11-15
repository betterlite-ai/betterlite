use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use serde_json::error::Category;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponseDocument, RerankResponseMeta, RerankResponseResult,
    RerankTokens,
};
use crate::api::rerank_contracts::wire::RerankContractError;
use crate::api::responses_shapes::wire::{ShapeDecodeError, decode_responses_shapes};
use crate::api::vector_store_records::records::score::Score;

use super::error::HostRerankError;
use super::{HostProvider, INFINITY_USAGE, JINA_USAGE, LIMITS};

#[derive(Deserialize)]
pub(super) struct Wire {
    pub(super) id: Option<String>,
    pub(super) usage: Option<IndexMap<String, Json>>,
    pub(super) results: Option<Vec<WireResult>>,
}

#[derive(Deserialize)]
pub(super) struct WireResult {
    index: i64,
    relevance_score: Number,
    // an absent document stays out of the map and an explicit null keeps the key as sent
    #[serde(flatten)]
    remaining: IndexMap<String, Json>,
}

enum Counted {
    Absent,
    Null,
    Count(i64),
}

impl Counted {
    const fn into_value(self) -> i64 {
        match self {
            Self::Absent | Self::Null => 0,
            Self::Count(count) => count,
        }
    }

    const fn is_present(&self) -> bool {
        match self {
            Self::Absent => false,
            Self::Null | Self::Count(_) => true,
        }
    }

    const fn count(self) -> Option<i64> {
        match self {
            Self::Absent | Self::Null => None,
            Self::Count(count) => Some(count),
        }
    }
}

// an absent counter stays out of the answer and an explicit null is sent as null
const fn counted_tokens(input: Counted, output: Counted) -> RerankTokens {
    RerankTokens {
        input_tokens: input.is_present().then_some(input.count()),
        output_tokens: output.is_present().then_some(output.count()),
    }
}

pub(super) fn decode_wire(status: u16, bytes: &[u8], text: &str) -> Result<Wire, HostRerankError> {
    decode_responses_shapes(bytes, LIMITS).map_err(|error| match error {
        ShapeDecodeError::Json(source) if source.classify() != Category::Data => {
            HostRerankError::Json {
                status_code: status,
                body: text.to_owned(),
                source,
            }
        }
        source @ (ShapeDecodeError::Json(_)
        | ShapeDecodeError::JsonTooLarge
        | ShapeDecodeError::TooManyItems
        | ShapeDecodeError::TooDeep) => HostRerankError::Decode { source },
    })
}

pub(super) fn usage_meta(
    provider: HostProvider,
    usage: IndexMap<String, Json>,
) -> Result<(RerankResponseMeta, BTreeMap<String, Json>), HostRerankError> {
    let search = counter(&usage, "search_units")?;
    let total = counter(&usage, "total_tokens")?;
    let billed_units = RerankBilledUnits {
        search_units: search.is_present().then_some(search.count()),
        total_tokens: total.is_present().then_some(total.count()),
    };
    let (tokens, consumed) = match provider {
        HostProvider::Jina => (jina_tokens(&usage)?, JINA_USAGE),
        HostProvider::Infinity => (infinity_tokens(&usage)?, INFINITY_USAGE),
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

fn jina_tokens(usage: &IndexMap<String, Json>) -> Result<RerankTokens, HostRerankError> {
    Ok(counted_tokens(
        counter(usage, "input_tokens")?,
        counter(usage, "output_tokens")?,
    ))
}

// LiteLLM subtracts the prompt from the total without clamping, so a total below the prompt is negative here too
fn infinity_tokens(usage: &IndexMap<String, Json>) -> Result<RerankTokens, HostRerankError> {
    let prompt = counter(usage, "prompt_tokens")?.into_value();
    let total = counter(usage, "total_tokens")?.into_value();
    let output = total
        .checked_sub(prompt)
        .ok_or(HostRerankError::CounterOverflow)?;
    Ok(counted_tokens(
        Counted::Count(prompt),
        Counted::Count(output),
    ))
}

fn counter(usage: &IndexMap<String, Json>, name: &'static str) -> Result<Counted, HostRerankError> {
    let Some(value) = usage.get(name) else {
        return Ok(Counted::Absent);
    };
    match value {
        Json::Null => Ok(Counted::Null),
        Json::Number(number) => number
            .as_i64()
            .map(Counted::Count)
            .ok_or(HostRerankError::Counter { name }),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(HostRerankError::Counter { name })
        }
    }
}

pub(super) fn result(
    provider: HostProvider,
    entry: &WireResult,
) -> Result<RerankResponseResult, HostRerankError> {
    let score = parse_decimal_lexeme(&entry.relevance_score.to_string())
        .map_err(RerankContractError::ScoreOutOfRange)?;
    let document = match (provider, entry.remaining.get("document")) {
        (_, None) | (HostProvider::Infinity, Some(Json::Null)) => None,
        (HostProvider::Jina, Some(Json::Null)) => {
            return Err(HostRerankError::from(RerankContractError::NullDocument));
        }
        (HostProvider::Infinity, Some(Json::Text(text))) if text.is_empty() => None,
        (_, Some(Json::Text(text))) => Some(RerankResponseDocument { text: text.clone() }),
        (HostProvider::Jina, Some(Json::Object(fields))) => {
            let Some(Json::Text(text)) = fields.get("text") else {
                return Err(HostRerankError::Document);
            };
            Some(RerankResponseDocument { text: text.clone() })
        }
        (_, Some(_)) => return Err(HostRerankError::Document),
    };
    Ok(RerankResponseResult {
        index: entry.index,
        relevance_score: Score(score),
        document,
    })
}
