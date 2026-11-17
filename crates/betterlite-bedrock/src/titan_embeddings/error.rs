use betterlite_prompts::arguments::json::Json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TitanEmbeddingsError {
    #[error("got multiple values for keyword argument '{0}'")]
    KeywordCollision(&'static str),
    #[error("the embedding reply holds a value that cannot be searched for a name")]
    NotIterable,
    #[error("the embedding reply holds a value that cannot be indexed by a name")]
    NotSubscriptable,
    #[error("No embedding data found in response: {0:?}")]
    NoEmbeddingData(Json),
    #[error("the embedding reply is missing '{0}'")]
    MissingField(&'static str),
    #[error("'{0}' is neither a list of numbers nor an encoded string")]
    MalformedEmbedding(&'static str),
    #[error("inputTextTokenCount is not a non-negative integer")]
    MalformedTokenCount,
    #[error("the embedding reply holds more records than an index can address")]
    IndexOverflow,
    #[error("the embedding token counts exceed the supported range")]
    TokenOverflow,
}
