use betterlite_prompts::definitions::error::DefinitionsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VertexSchemaError {
    #[error("the schema nests deeper than the {limit} level recursion limit")]
    DepthLimitExceeded { limit: i64 },
    #[error("an anyOf schema with only a null branch is not supported")]
    NullOnlyAnyOf,
    #[error("the $defs table is not an object")]
    DefinitionsNotObject,
    #[error(transparent)]
    Definitions {
        #[from]
        source: DefinitionsError,
    },
}
