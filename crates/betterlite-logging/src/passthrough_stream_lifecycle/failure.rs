use std::error::Error;
use std::fmt::{Display, Formatter};
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

#[derive(Debug, Clone)]
pub struct FailureCause(Arc<dyn Error + Send + Sync>);

impl Display for FailureCause {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.0.as_ref(), formatter)
    }
}

impl Error for FailureCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

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

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::sync::Arc;

    use super::StreamFailure;

    #[test]
    fn failure_source_chain_preserves_upstream_cause() {
        let failure = StreamFailure::upstream(
            "UpstreamReadError".to_owned(),
            "read failed".to_owned(),
            "Traceback: upstream".to_owned(),
            Arc::new(std::io::Error::other("socket reset")),
        );
        let source = failure.source().and_then(|error| error.source()).unwrap();
        assert_eq!(source.to_string(), "socket reset");
        assert_eq!(failure.to_string(), "read failed");
        assert_eq!(failure.class(), "UpstreamReadError");
        assert_eq!(failure.message(), "read failed");
        assert_eq!(failure.traceback(), "Traceback: upstream");
    }
}
