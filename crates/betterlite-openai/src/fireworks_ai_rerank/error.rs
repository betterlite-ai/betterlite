use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use thiserror::Error;

use crate::fireworks_ai_common::error::FireworksError;

#[derive(Debug, Error)]
pub enum FireworksRerankError {
    #[error(
        "FIREWORKS_API_KEY is not set. Please set 'FIREWORKS_API_KEY' or 'FIREWORKS_AI_API_KEY' in your environment"
    )]
    MissingApiKey,
    #[error("query is required for Fireworks AI rerank")]
    MissingQuery,
    #[error("documents are required for Fireworks AI rerank")]
    MissingDocuments,
    #[error("the rerank response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the rerank response is not valid JSON")]
    ResponseDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the rerank response has no results")]
    NoResults,
    #[error("a rerank result has no field `{field}`")]
    MissingResultField { field: &'static str },
    #[error("a rerank result index must be an integer")]
    IndexShape,
    #[error("a rerank result relevance_score cannot be represented exactly")]
    RelevanceScore {
        #[source]
        source: DecimalLexemeError,
    },
    #[error(transparent)]
    Fireworks(#[from] FireworksError),
}
