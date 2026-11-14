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
    let Json::Object(root) = serde_json::from_slice::<Json>(bytes)? else {
        return Err(RerankDecodeError::NotAnObject);
    };
    let items = match root.get("results") {
        Some(Json::List(items)) if !items.is_empty() => items,
        Some(Json::List(_)) | None | Some(Json::Null) => return Err(RerankDecodeError::NoResults),
        Some(_) => return Err(RerankDecodeError::Field { field: "results" }),
    };
    // an absent or null id takes the generated one; an explicit string is used as sent
    let id = match root.get("id") {
        None | Some(Json::Null) => generated_id.to_owned(),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(RerankDecodeError::Field { field: "id" }),
    };
    let usage = match root.get("usage") {
        None => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(RerankDecodeError::Field { field: "usage" }),
    };
    Ok(RerankResponse {
        id: Some(id),
        results: Some(items.iter().map(result).collect::<Result<_, _>>()?),
        meta: Some(meta(usage)?),
        hidden_params: BTreeMap::new(),
    })
}

fn result(value: &Json) -> Result<RerankResponseResult, RerankDecodeError> {
    let Json::Object(fields) = value else {
        return Err(RerankDecodeError::Field { field: "results" });
    };
    Ok(RerankResponseResult {
        index: required_index(fields)?,
        relevance_score: required_score(fields)?,
        document: None,
    })
}

fn required_index(fields: &IndexMap<String, Json>) -> Result<i64, RerankDecodeError> {
    let Some(Json::Number(number)) = fields.get("index") else {
        return Err(RerankDecodeError::Field { field: "index" });
    };
    number
        .as_i64()
        .ok_or(RerankDecodeError::Field { field: "index" })
}

fn required_score(fields: &IndexMap<String, Json>) -> Result<Score, RerankDecodeError> {
    let Some(Json::Number(number)) = fields.get("relevanceScore") else {
        return Err(RerankDecodeError::Field {
            field: "relevanceScore",
        });
    };
    Ok(Score::try_from(number.clone())?)
}

// LiteLLM reads the same usage mapping into both records; absent usage gets one default search unit
fn meta(usage: Option<&IndexMap<String, Json>>) -> Result<RerankResponseMeta, RerankDecodeError> {
    let (search_units, total_tokens, input_tokens, output_tokens) = match usage {
        None => (Some(Some(1)), None, None, None),
        Some(fields) => (
            integer(fields, "search_units")?,
            integer(fields, "total_tokens")?,
            integer(fields, "input_tokens")?,
            integer(fields, "output_tokens")?,
        ),
    };
    Ok(RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(RerankBilledUnits {
            search_units,
            total_tokens,
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens,
            output_tokens,
        })),
    })
}

fn integer(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<Option<i64>>, RerankDecodeError> {
    let invalid = || RerankDecodeError::Field { field: name };
    match fields.get(name) {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(None)),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(|value| Some(Some(value)))
            .ok_or_else(invalid),
        Some(_) => Err(invalid()),
    }
}
