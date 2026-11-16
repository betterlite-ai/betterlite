use std::error::Error as StdError;

use betterlite_core::api::messages_native_stream::retention::RetainedOutputLimit;
use thiserror::Error;

use super::error::NomaLifecycleError;
use crate::cato::error::ModelStreamError;

// The assembler is injected, so its own error type rides along as the generic parameter
#[derive(Debug, Error)]
pub enum NomaStreamError<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Lifecycle(#[from] NomaLifecycleError),
    #[error("the model stream failed")]
    Model {
        #[source]
        source: ModelStreamError,
    },
    #[error("the streamed chunks could not be assembled")]
    Assembly {
        #[source]
        source: E,
    },
    #[error("the streamed response is over the buffer budget of {bytes} bytes and {chunks} chunks")]
    OverBudget {
        bytes: usize,
        chunks: usize,
        #[source]
        source: RetainedOutputLimit,
    },
}
