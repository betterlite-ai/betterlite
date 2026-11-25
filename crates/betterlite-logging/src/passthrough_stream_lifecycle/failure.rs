use std::error::Error;
use std::sync::Arc;

use crate::passthrough_stream_logging::object::StreamLoggingObject;

use super::ports::StreamWork;

#[derive(Debug, Clone, thiserror::Error)]
pub enum StreamFailure {
    #[error("{message}")]
    Upstream {
        class: String,
        message: String,
        traceback: String,
        #[source]
        cause: FailureCause,
    },
}

#[derive(Debug, Clone, thiserror::Error)]
#[error(transparent)]
pub struct FailureCause(Arc<dyn Error + Send + Sync>);

impl StreamFailure {
    pub fn upstream(
        class: String,
        message: String,
        traceback: String,
        cause: Arc<dyn Error + Send + Sync>,
    ) -> Self {
        Self::Upstream {
            class,
            message,
            traceback,
            cause: FailureCause(cause),
        }
    }

    pub fn class(&self) -> &str {
        match self {
            Self::Upstream { class, .. } => class,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Upstream { message, .. } => message,
        }
    }

    pub fn traceback(&self) -> &str {
        match self {
            Self::Upstream { traceback, .. } => traceback,
        }
    }
}

// litellm_logging_obj.dispatch_failure_handlers(exception, traceback.format_exc(), prefer_async_handlers=True)
pub trait StreamFailureDispatch: Send + Sync {
    fn dispatch_failure(
        &self,
        logging_obj: Arc<StreamLoggingObject>,
        failure: &StreamFailure,
    ) -> StreamWork;
}

mod tests;
