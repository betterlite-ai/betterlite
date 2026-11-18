use betterlite_logging::logging_success::types::SuccessRequest;
use tokio::task::JoinHandle;

// The hook latches LiteLLM keeps beside the stream state: the chunk-hook marker, the pending
// logging tasks await_pending_logging drains, and the dispatch parked for the deferred stream end.
#[derive(Default)]
pub struct ResponsesStreamHookState {
    pub post_streaming_hooks_ran: bool,
    pub pending_logging_tasks: Vec<JoinHandle<()>>,
    pub deferred_stream_complete_args: Option<SuccessRequest>,
}
