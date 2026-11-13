use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::selection::SelectionError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbeddingInputError {
    #[error("the tokenizer for {model} could not be selected")]
    Select {
        model: String,
        #[source]
        source: Box<SelectionError>,
    },
    #[error("the embedding prompt could not be encoded")]
    Encode {
        #[source]
        source: EngineError,
    },
    #[error("the truncated embedding prompt could not be decoded")]
    Decode {
        #[source]
        source: EngineError,
    },
}
