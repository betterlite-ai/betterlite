use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseMeta, RerankResponseResult, RerankTokens,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RerankDecodeError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the rerank reply is not a JSON object")]
    NotAnObject,
    #[error("the rerank reply has no results")]
    NoResults,
    #[error("the rerank reply field `{field}` is not valid")]
    Field { field: &'static str },
    #[error("the rerank reply score cannot be represented exactly")]
    Score(#[from] DecimalLexemeError),
}

pub fn decode_rerank_reply(
    bytes: &[u8],
    generated_id: &str,
) -> Result<RerankResponse, RerankDecodeError> {
    let parsed = serde_json::from_slice::<Json>(bytes)?;
    let Json::Object(root) = parsed else {
        return Err(RerankDecodeError::NotAnObject);
    };
    let usage = match root.get("usage") {
        None => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => {
            return Err(RerankDecodeError::Field { field: "usage" });
        }
    };
    let results = results(&root)?;
    Ok(RerankResponse {
        id: Some(reply_id(&root, generated_id)?),
        results: Some(results),
        meta: Some(meta(usage)?),
        hidden_params: BTreeMap::new(),
    })
}

fn results(root: &IndexMap<String, Json>) -> Result<Vec<RerankResponseResult>, RerankDecodeError> {
    match root.get("results") {
        Some(Json::List(items)) if items.is_empty() => Err(RerankDecodeError::NoResults),
        Some(Json::List(items)) => items.iter().map(result).collect(),
        None | Some(Json::Null) => Err(RerankDecodeError::NoResults),
        Some(_) => Err(RerankDecodeError::Field { field: "results" }),
    }
}

fn result(value: &Json) -> Result<RerankResponseResult, RerankDecodeError> {
    let Json::Object(fields) = value else {
        return Err(RerankDecodeError::Field { field: "results" });
    };
    let index = required_index(fields)?;
    let relevance_score = required_score(fields)?;
    Ok(RerankResponseResult {
        index,
        relevance_score,
        document: None,
    })
}

fn required_index(fields: &IndexMap<String, Json>) -> Result<i64, RerankDecodeError> {
    match fields.get("index") {
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(RerankDecodeError::Field { field: "index" }),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RerankDecodeError::Field { field: "index" })
        }
    }
}

fn required_score(fields: &IndexMap<String, Json>) -> Result<Score, RerankDecodeError> {
    match fields.get("relevanceScore") {
        Some(Json::Number(number)) => Ok(Score::try_from(number.clone())?),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RerankDecodeError::Field {
                field: "relevanceScore",
            })
        }
    }
}

// Python's `response.get("id") or uuid4()` takes a falsy id from the generated one
fn reply_id(root: &IndexMap<String, Json>, generated: &str) -> Result<String, RerankDecodeError> {
    match root.get("id") {
        None | Some(Json::Null) => Ok(generated.to_owned()),
        Some(Json::Text(text)) if text.is_empty() => Ok(generated.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(false)) => Ok(generated.to_owned()),
        Some(Json::Number(number)) if is_zero(number) => Ok(generated.to_owned()),
        Some(Json::List(items)) if items.is_empty() => Ok(generated.to_owned()),
        Some(Json::Object(fields)) if fields.is_empty() => Ok(generated.to_owned()),
        Some(_) => Err(RerankDecodeError::Field { field: "id" }),
    }
}

fn is_zero(number: &serde_json::Number) -> bool {
    number.as_i64() == Some(0) || number.as_f64() == Some(0.0)
}

fn meta(usage: Option<&IndexMap<String, Json>>) -> Result<RerankResponseMeta, RerankDecodeError> {
    let (billed_units, tokens) = match usage {
        None => (
            RerankBilledUnits {
                search_units: Some(Some(1)),
                total_tokens: None,
            },
            RerankTokens {
                input_tokens: None,
                output_tokens: None,
            },
        ),
        Some(fields) => (
            RerankBilledUnits {
                search_units: integer(fields, "search_units")?,
                total_tokens: integer(fields, "total_tokens")?,
            },
            RerankTokens {
                input_tokens: integer(fields, "input_tokens")?,
                output_tokens: integer(fields, "output_tokens")?,
            },
        ),
    };
    Ok(RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(billed_units)),
        tokens: Some(Some(tokens)),
    })
}

fn integer(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<Option<i64>>, RerankDecodeError> {
    match fields.get(name) {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(None)),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(|value| Some(Some(value)))
            .ok_or(RerankDecodeError::Field { field: name }),
        Some(_) => Err(RerankDecodeError::Field { field: name }),
    }
}
