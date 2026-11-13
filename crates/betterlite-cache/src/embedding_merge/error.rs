use thiserror::Error;

use crate::handler::HandlerError;

#[derive(Debug, Error)]
pub enum MergeError {
    #[error("the {field} total does not fit a number")]
    Overflow { field: &'static str },
    #[error("the cached results cover {cached} positions but the input has {input}")]
    CachedLength { cached: usize, input: usize },
    #[error("the fresh embeddings ran out before position {position}")]
    MissingFresh { position: usize },
    #[error("the response time does not fit a number")]
    ResponseTime,
    #[error("the {field} value must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the embedding input could not be read")]
    Input {
        #[from]
        source: HandlerError,
    },
}

#[derive(Debug, Error)]
pub enum ProcessError<Service> {
    #[error("the cached embeddings could not be merged")]
    Merge {
        #[from]
        source: MergeError,
    },
    #[error("a gateway service failed while the cached embeddings were processed")]
    Service {
        #[source]
        source: Service,
    },
}
