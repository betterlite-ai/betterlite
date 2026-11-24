use std::sync::Arc;

use crate::passthrough_stream_logging::object::StreamLoggingObject;

use super::ports::StreamWork;

// The Python exception a failed upstream read raised, as the failure handlers log it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFailure {
    pub class: String,
    pub message: String,
    // traceback.format_exc(), captured where the exception was raised
    pub traceback: String,
}

// litellm_logging_obj.dispatch_failure_handlers(exception, traceback.format_exc(), prefer_async_handlers=True)
pub trait StreamFailureDispatch: Send + Sync {
    fn dispatch_failure(
        &self,
        logging_obj: Arc<StreamLoggingObject>,
        failure: &StreamFailure,
    ) -> StreamWork;
}
