use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;

use crate::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, StreamDefaults,
};

// The route's logging port. The gateway's LoggingState pipeline (B188) replaces it once a text call owns one;
// until then the record goes out with counts and no message text, so no prompt reaches a log.
pub struct TracingCompletionLog;

impl CompletionLogger for TracingCompletionLog {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        tracing::debug!(
            messages = record.input.len(),
            headers = record.headers.0.len(),
            api_base = ?record.api_base,
            "text completion pre-call"
        );
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        tracing::debug!(kind = ?record.kind, "text completion post-call");
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
        }
    }
}
