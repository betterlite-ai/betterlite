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
        Some(Json::List(_) | Json::Null) | None => return Err(RerankDecodeError::NoResults),
        Some(_) => return Err(RerankDecodeError::Field { field: "results" }),
    };
    // an absent or null id takes the generated one; an explicit string is used as sent
    let id = match root.get("id") {
        None | Some(Json::Null) => generated_id.to_owned(),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => return Err(RerankDecodeError::Field { field: "id" }),
    };
    // an absent or null usage takes the default, as docs/compatibility.md says for every key
    let usage = match root.get("usage") {
        None | Some(Json::Null) => None,
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

// abs, null and a count are the three states pydantic's optional field keeps apart
enum Quota {
    Absent,
    Null,
    Count(i64),
}

impl From<Quota> for Option<Option<i64>> {
    fn from(quota: Quota) -> Self {
        match quota {
            Quota::Absent => None,
            Quota::Null => Some(None),
            Quota::Count(count) => Some(Some(count)),
        }
    }
}

// LiteLLM reads the same usage mapping into both records; absent usage gets one default search unit
fn meta(usage: Option<&IndexMap<String, Json>>) -> Result<RerankResponseMeta, RerankDecodeError> {
    let (search_units, total_tokens, input_tokens, output_tokens) = match usage {
        None => (Quota::Count(1), Quota::Absent, Quota::Absent, Quota::Absent),
        Some(fields) => (
            quota(fields, "search_units")?,
            quota(fields, "total_tokens")?,
            quota(fields, "input_tokens")?,
            quota(fields, "output_tokens")?,
        ),
    };
    Ok(RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(RerankBilledUnits {
            search_units: Option::from(search_units),
            total_tokens: Option::from(total_tokens),
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens: Option::from(input_tokens),
            output_tokens: Option::from(output_tokens),
        })),
    })
}

fn quota(fields: &IndexMap<String, Json>, name: &'static str) -> Result<Quota, RerankDecodeError> {
    match fields.get(name) {
        None => Ok(Quota::Absent),
        Some(Json::Null) => Ok(Quota::Null),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Quota::Count)
            .ok_or(RerankDecodeError::Field { field: name }),
        Some(_) => Err(RerankDecodeError::Field { field: name }),
    }
}
