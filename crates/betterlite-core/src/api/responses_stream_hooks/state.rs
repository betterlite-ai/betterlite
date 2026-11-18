use betterlite_logging::logging_success::types::SuccessRequest;
use tokio::task::JoinHandle;

#[derive(Default)]
pub struct ResponsesStreamHookState {
    pub post_streaming_hooks_ran: bool,
    pub pending_logging_tasks: Vec<JoinHandle<()>>,
    pub deferred_stream_complete_args: Option<SuccessRequest>,
}
