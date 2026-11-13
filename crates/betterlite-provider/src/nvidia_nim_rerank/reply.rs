use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use serde::Deserialize;
use uuid::Uuid;

use super::NvidiaNimRerankError;
use crate::api::rerank_contracts::Params;

const PASSAGES: &str = "passages";
const TEXT: &str = "text";

#[derive(Debug, Clone, Deserialize)]
pub struct NvidiaNimRerankReply {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub rankings: Vec<NvidiaNimRanking>,
    #[serde(default)]
    pub usage: Option<NvidiaNimUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NvidiaNimRanking {
    pub index: i64,
    pub logit: Score,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NvidiaNimUsage {
    #[serde(default)]
    pub total_tokens: Option<i64>,
}

pub fn decode_nvidia_nim_rerank_reply(
    bytes: &[u8],
) -> Result<NvidiaNimRerankReply, NvidiaNimRerankError> {
    match serde_json::from_slice(bytes) {
        Ok(reply) => Ok(reply),
        Err(source) => Err(NvidiaNimRerankError::from_json(source)),
    }
}

pub fn to_response(
    reply: &NvidiaNimRerankReply,
    request_data: &Params,
    new_id: fn() -> Uuid,
) -> Result<RerankResponse, NvidiaNimRerankError> {
    let passages: &[Json] = match request_data.get(PASSAGES) {
        Some(Json::List(items)) => items.as_slice(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => &[],
    };
    let mut results = Vec::with_capacity(reply.rankings.len());
    for ranking in &reply.rankings {
        let document = passage_text(passages, ranking.index)?;
        results.push(RerankResponseResult {
            index: ranking.index,
            relevance_score: ranking.logit.clone(),
            document: document.map(|text| RerankResponseDocument {
                text: text.to_owned(),
            }),
        });
    }
    let counted = i64::try_from(results.len()).unwrap_or(i64::MAX);
    let total_tokens = reply
        .usage
        .as_ref()
        .and_then(|usage| usage.total_tokens)
        .filter(|tokens| *tokens > 0)
        .unwrap_or(counted);
    Ok(RerankResponse {
        id: Some(reply.id.clone().unwrap_or_else(|| (new_id)().to_string())),
        results: Some(results),
        meta: Some(RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(RerankBilledUnits {
                search_units: None,
                total_tokens: Some(Some(total_tokens)),
            })),
            tokens: None,
        }),
        hidden_params: BTreeMap::new(),
    })
}

fn passage_text<'passages>(
    passages: &'passages [Json],
    index: i64,
) -> Result<Option<&'passages str>, NvidiaNimRerankError> {
    let position = if index < 0 {
        let offset = usize::try_from(index.unsigned_abs()).map_err(|_| {
            NvidiaNimRerankError::RankingIndex {
                index,
                passages: passages.len(),
            }
        })?;
        passages
            .len()
            .checked_sub(offset)
            .ok_or(NvidiaNimRerankError::RankingIndex {
                index,
                passages: passages.len(),
            })?
    } else {
        usize::try_from(index).map_err(|_| NvidiaNimRerankError::RankingIndex {
            index,
            passages: passages.len(),
        })?
    };
    let Some(Json::Object(fields)) = passages.get(position) else {
        return Ok(None);
    };
    match fields.get(TEXT) {
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NvidiaNimRerankError::DocumentTextNotString)
        }
        None => Ok(None),
    }
}
