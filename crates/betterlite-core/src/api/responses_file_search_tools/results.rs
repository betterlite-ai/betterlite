use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use indexmap::IndexMap;
use rust_decimal::RoundingStrategy;
use serde::Serialize;

use crate::api::responses_tool_namespaces::values::{Fields, has_value};

use super::error::FileSearchToolsError;
use super::wire;

pub const NO_RESULTS: &str = "No results found in the vector store.";
const SCORE_PLACES: u32 = 3;

// the include projection of one search result, in the key order OpenAI's file_search_call.search_results uses
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResultRecord {
    pub file_id: String,
    pub filename: String,
    pub score: Option<Score>,
    pub text: String,
    pub attributes: Json,
}

// _get_field read with no default argument: an absent key or attribute reads as null, like Python's None
pub const NO_DEFAULT: Json = Json::Null;

// _get_field: a dict key that is present but null reads as null, and only a missing key or attribute takes the default
#[must_use]
pub fn field(fields: &Fields, key: &str, default: &Json) -> Json {
    fields.get(key).cloned().unwrap_or_else(|| default.clone())
}

#[must_use]
pub fn joined_content_text(result: &VectorStoreSearchResult) -> String {
    let chunks = result.content.as_deref().unwrap_or_default();
    let texts: Vec<&str> = chunks
        .iter()
        .filter_map(|chunk| chunk.text.as_deref())
        .filter(|text| !text.is_empty())
        .collect();
    texts.join(" ")
}

#[must_use]
pub fn format_results_as_tool_output(results: &[VectorStoreSearchResult]) -> String {
    if results.is_empty() {
        return NO_RESULTS.to_owned();
    }
    let parts: Vec<String> = results
        .iter()
        .enumerate()
        .map(|(position, result)| {
            let index = position.saturating_add(1);
            format!(
                "{}\n{}",
                result_header(index, result),
                joined_content_text(result)
            )
        })
        .collect();
    parts.join("\n\n")
}

// falsey filename and file_id drop their segment, while a present score always renders with three places
fn result_header(index: usize, result: &VectorStoreSearchResult) -> String {
    let mut header = format!("[Result {index}");
    if let Some(filename) = result.filename.as_deref().filter(|text| !text.is_empty()) {
        header.push_str(" | ");
        header.push_str(filename);
    }
    if let Some(file_id) = result.file_id.as_deref().filter(|text| !text.is_empty()) {
        header.push_str(" | file_id=");
        header.push_str(file_id);
    }
    if let Some(score) = &result.score {
        let rounded = score
            .0
            .round_dp_with_strategy(SCORE_PLACES, RoundingStrategy::MidpointNearestEven);
        let segment = format!(" | score={rounded:.3}");
        header.push_str(&segment);
    }
    header.push(']');
    header
}

// all chunks stay in input order: repeated results are kept, as OpenAI's native file_search surfaces every chunk
pub(super) fn include_record(
    result: &VectorStoreSearchResult,
) -> Result<SearchResultRecord, FileSearchToolsError> {
    let attributes = match result.attributes.as_ref() {
        Some(value) => wire(value)?,
        None => Json::Object(IndexMap::new()),
    };
    Ok(SearchResultRecord {
        file_id: result.file_id.clone().unwrap_or_default(),
        filename: result.filename.clone().unwrap_or_default(),
        score: result.score.clone(),
        text: joined_content_text(result),
        attributes: if has_value(&attributes) {
            attributes
        } else {
            Json::Object(IndexMap::new())
        },
    })
}
