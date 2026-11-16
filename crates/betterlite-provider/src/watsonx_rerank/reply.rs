use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use serde::Deserialize;
use serde_json::error::Category;

use super::error::WatsonxRerankError;

#[derive(Debug, Deserialize)]
pub struct WatsonxRerankReply {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub results: Option<Vec<WatsonxRerankResult>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub input_token_count: Option<Option<i64>>,
}

#[derive(Debug, Deserialize)]
pub struct WatsonxRerankResult {
    pub index: i64,
    pub score: Score,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub input: Option<Option<Json>>,
}

// a reply that is valid JSON but not an object must fail, where serde would read an array as the struct's fields
pub fn decode_watsonx_rerank_reply(bytes: &[u8]) -> Result<WatsonxRerankReply, WatsonxRerankError> {
    let classify = |source: serde_json::Error| match source.classify() {
        Category::Data => WatsonxRerankError::Reply(source),
        Category::Io | Category::Syntax | Category::Eof => WatsonxRerankError::Parse(source),
    };
    serde_json::from_slice::<BTreeMap<String, Json>>(bytes).map_err(classify)?;
    serde_json::from_slice(bytes).map_err(classify)
}
