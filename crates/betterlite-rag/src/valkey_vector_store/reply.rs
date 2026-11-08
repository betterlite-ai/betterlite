#![expect(
    clippy::float_arithmetic,
    reason = "LiteLLM's result score is 1.0 minus the float KNN distance valkey-search returns"
)]

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use rust_decimal::Decimal;

use super::error::ValkeyError;
use super::ports::ValkeyDocument;
use super::query::DISTANCE_FIELD;

pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

pub fn search_response(
    documents: &[ValkeyDocument],
    query_text: &str,
    text_field: &str,
) -> Result<VectorStoreSearchResponse, ValkeyError> {
    let mut results = Vec::with_capacity(documents.len());
    for document in documents {
        results.push(search_result(document, text_field)?);
    }
    results.sort_by_key(|result| {
        Reverse(result.score.as_ref().map_or(Decimal::ZERO, |score| score.0))
    });
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(query_text.to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn search_result(
    document: &ValkeyDocument,
    text_field: &str,
) -> Result<VectorStoreSearchResult, ValkeyError> {
    let distance =
        document
            .fields
            .get(DISTANCE_FIELD)
            .ok_or_else(|| ValkeyError::MissingDistance {
                id: document.id.clone(),
            })?;
    let score = score_from_distance(distance, &document.id)?;
    Ok(VectorStoreSearchResult {
        score: Some(Score(score)),
        content: Some(vec![VectorStoreResultContent {
            text: Some(document.fields.get(text_field).cloned().unwrap_or_default()),
            type_name: Some("text".to_owned()),
            extra: BTreeMap::new(),
        }]),
        file_id: Some(document.id.clone()),
        filename: Some(document.id.clone()),
        attributes: None,
        extra: BTreeMap::new(),
    })
}

fn score_from_distance(distance: &str, id: &str) -> Result<Decimal, ValkeyError> {
    let invalid = || ValkeyError::InvalidDistance {
        id: id.to_owned(),
        value: distance.to_owned(),
    };
    let parsed = distance.parse::<f64>().map_err(|_source| invalid())?;
    let score = 1.0_f64 - parsed;
    Decimal::from_str(&format!("{score:?}")).map_err(|_source| invalid())
}
