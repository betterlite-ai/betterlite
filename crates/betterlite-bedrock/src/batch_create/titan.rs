use betterlite_models::decisions::Json;
use serde::Serialize;

const EMBEDDING_KEY: &str = "embedding";
const EMBEDDINGS_BY_TYPE_KEY: &str = "embeddingsByType";
const INPUT_TEXT_TOKEN_COUNT: &str = "inputTextTokenCount";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TitanBatchUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[must_use]
pub fn titan_batch_token_usage(output: &Json) -> Option<TitanBatchUsage> {
    let Json::Object(fields) = output else {
        return None;
    };
    if !fields.contains_key(EMBEDDING_KEY) && !fields.contains_key(EMBEDDINGS_BY_TYPE_KEY) {
        return None;
    }
    let Some(Json::Number(number)) = fields.get(INPUT_TEXT_TOKEN_COUNT) else {
        return None;
    };
    let count = number.as_i64()?;
    Some(TitanBatchUsage {
        prompt_tokens: count,
        completion_tokens: 0,
        total_tokens: count,
    })
}
