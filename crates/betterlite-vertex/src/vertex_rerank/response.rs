use std::cmp::Reverse;
use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseMeta, RerankResponseResult,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

use super::error::VertexRerankError;
use super::request::RankRequest;

const RECORDS_PER_SEARCH_UNIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedRecord {
    pub index: i64,
    pub score: Score,
}

// serde's derive would also read a list by position, and LiteLLM's .get() refuses a non-object root
pub fn decode_vertex_rerank(bytes: &[u8]) -> Result<Vec<RankedRecord>, VertexRerankError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(VertexRerankError::TooLarge { size: bytes.len() });
    }
    let root: Json = serde_json::from_slice(bytes).map_err(VertexRerankError::Json)?;
    let Json::Object(mut envelope) = root else {
        return Err(VertexRerankError::NotObject);
    };
    let records = match envelope.shift_remove("records") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(records)) => records,
        Some(_) => return Err(VertexRerankError::RecordsNotList),
    };
    let mut ranked = Vec::with_capacity(records.len());
    for (position, record) in records.iter().enumerate() {
        let Json::Object(fields) = record else {
            return Err(VertexRerankError::RecordNotObject { position });
        };
        ranked.push(ranked_record(position, fields)?);
    }
    Ok(ranked)
}

fn ranked_record(
    position: usize,
    fields: &IndexMap<String, Json>,
) -> Result<RankedRecord, VertexRerankError> {
    let index = match fields.get("id") {
        Some(Json::Text(text)) => text.trim().parse::<i64>().ok(),
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_)) | None => None,
    }
    .ok_or(VertexRerankError::RecordId { position })?;
    let score = match fields.get("score") {
        None | Some(Json::Null) => Score(Decimal::ONE),
        Some(Json::Number(number)) => Score::try_from(number.clone())
            .map_err(|source| VertexRerankError::Score { position, source })?,
        Some(_) => return Err(VertexRerankError::ScoreType { position }),
    };
    Ok(RankedRecord { index, score })
}

#[must_use]
pub fn transform_response(
    mut records: Vec<RankedRecord>,
    request: &RankRequest,
    id: Uuid,
) -> RerankResponse {
    records.sort_by_key(|record| Reverse(record.score.0));
    let results = records
        .into_iter()
        .map(|record| RerankResponseResult {
            index: record.index,
            relevance_score: record.score,
            document: None,
        })
        .collect();
    let search_units =
        i64::try_from(request.records.len().div_ceil(RECORDS_PER_SEARCH_UNIT)).unwrap_or(i64::MAX);
    RerankResponse {
        id: Some(format!("vertex_ai_rerank_{id}")),
        results: Some(results),
        meta: Some(RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(RerankBilledUnits {
                search_units: Some(Some(search_units)),
                total_tokens: None,
            })),
            tokens: None,
        }),
        hidden_params: BTreeMap::new(),
    }
}
