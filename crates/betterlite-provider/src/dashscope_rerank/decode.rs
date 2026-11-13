use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::dashscope_embedding::error::DashscopeError;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::api::rerank_contracts::Headers;

// a reply the rerank records cannot hold is an unclassified provider failure, which LiteLLM answers with 500
const SHAPE_FAILURE_STATUS: u16 = 500;

pub fn decode_dashscope_rerank_reply(
    bytes: &[u8],
    status_code: u16,
) -> Result<RerankResponse, DashscopeError> {
    let body: Json = serde_json::from_slice(bytes).map_err(|_| DashscopeError {
        status_code,
        message: body_text(bytes),
        headers: Headers::new(),
    })?;
    let Json::Object(fields) = &body else {
        return Err(shape_error("the reply"));
    };
    let results = fields.get("results");
    if fields.contains_key("code") && results.is_none() {
        return Err(DashscopeError {
            status_code,
            message: code_message(fields, &body),
            headers: Headers::new(),
        });
    }
    let items = match results {
        None | Some(Json::Null) => return Err(no_results_error(status_code, &body)),
        Some(Json::List(items)) => items,
        Some(_) => return Err(shape_error("results")),
    };
    let mut transformed = Vec::with_capacity(items.len());
    for item in items {
        transformed.push(rerank_result(item)?);
    }
    let total_tokens = total_tokens(fields.get("usage"))?;
    let id = match fields.get("id") {
        Some(Json::Text(text)) if !text.is_empty() => text.clone(),
        _ => Uuid::new_v4().to_string(),
    };
    let meta = match total_tokens {
        Some(total) => RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(RerankBilledUnits {
                search_units: None,
                total_tokens: Some(Some(total)),
            })),
            tokens: Some(Some(RerankTokens {
                input_tokens: Some(Some(total)),
                output_tokens: None,
            })),
        },
        None => RerankResponseMeta::default(),
    };
    Ok(RerankResponse {
        id: Some(id),
        results: Some(transformed),
        meta: Some(meta),
        hidden_params: BTreeMap::new(),
    })
}

fn rerank_result(item: &Json) -> Result<RerankResponseResult, DashscopeError> {
    let Json::Object(fields) = item else {
        return Err(shape_error("results"));
    };
    let index = match fields.get("index") {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    };
    let Some(index) = index else {
        return Err(shape_error("results[].index"));
    };
    let relevance_score = match fields.get("relevance_score") {
        Some(Json::Number(number)) => Score::try_from(number.clone()).ok(),
        _ => None,
    };
    let Some(relevance_score) = relevance_score else {
        return Err(shape_error("results[].relevance_score"));
    };
    let document = match fields.get("document") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(RerankResponseDocument { text: text.clone() }),
        Some(Json::Object(document)) => match document.get("text") {
            Some(Json::Text(text)) => Some(RerankResponseDocument { text: text.clone() }),
            _ => return Err(shape_error("results[].document.text")),
        },
        Some(_) => None,
    };
    Ok(RerankResponseResult {
        index,
        relevance_score,
        document,
    })
}

fn total_tokens(usage: Option<&Json>) -> Result<Option<i64>, DashscopeError> {
    match usage {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => match fields.get("total_tokens") {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Number(number)) => number
                .as_i64()
                .map(Some)
                .ok_or_else(|| shape_error("usage.total_tokens")),
            Some(_) => Err(shape_error("usage.total_tokens")),
        },
        Some(_) => Err(shape_error("usage")),
    }
}

fn code_message(fields: &IndexMap<String, Json>, body: &Json) -> String {
    match fields.get("message") {
        Some(Json::Text(message)) => message.clone(),
        _ => serde_json::to_string(body).unwrap_or_default(),
    }
}

fn no_results_error(status_code: u16, body: &Json) -> DashscopeError {
    DashscopeError {
        status_code,
        message: format!(
            "No results in DashScope rerank response: {}",
            serde_json::to_string(body).unwrap_or_default()
        ),
        headers: Headers::new(),
    }
}

fn body_text(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).map_or_else(
        |_| "the DashScope rerank reply is not UTF-8 text".to_owned(),
        str::to_owned,
    )
}

fn shape_error(field: &str) -> DashscopeError {
    DashscopeError {
        status_code: SHAPE_FAILURE_STATUS,
        message: format!("the DashScope rerank reply field `{field}` has the wrong type"),
        headers: Headers::new(),
    }
}
