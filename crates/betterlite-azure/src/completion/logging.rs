use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;

// B249's pre-call and post-call dispatch, plus what CustomStreamWrapper reads off the logging object
pub trait CompletionHooks {
    fn pre_call(&mut self, call: PreCall) -> Result<(), PrePostError>;

    fn post_call(&mut self, call: PostCall);

    fn stream_defaults(&self) -> StreamDefaults;
}
