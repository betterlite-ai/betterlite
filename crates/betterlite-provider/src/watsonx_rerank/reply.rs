use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use serde::Deserialize;

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

pub fn decode_watsonx_rerank_reply(bytes: &[u8]) -> Result<WatsonxRerankReply, WatsonxRerankError> {
    serde_json::from_slice(bytes).map_err(|source| match source.classify() {
        serde_json::error::Category::Data => WatsonxRerankError::Reply(source),
        serde_json::error::Category::Io
        | serde_json::error::Category::Syntax
        | serde_json::error::Category::Eof => WatsonxRerankError::Parse(source),
    })
}
