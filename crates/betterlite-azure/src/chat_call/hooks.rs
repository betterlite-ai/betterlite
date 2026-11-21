use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use indexmap::IndexMap;

use crate::azure_stream_errors::records::PreCallRecord;

// B249's pre-call and post-call dispatch, plus what the streaming wrapper reads off the logging object
pub trait ChatHooks: Send {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    fn post_call(&mut self, call: PostCall);

    // logging_obj.model_call_details["response_headers"] = headers
    fn response_headers(&mut self, headers: &IndexMap<String, String>);

    fn stream_defaults(&self) -> StreamDefaults;
}
