use betterlite_azure::completion::logging::CompletionHooks;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;

use super::view;

// The logging object the handler calls: the oracle records the pre and post calls instead of emitting them
#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<Json>,
    pub(super) post: Vec<Json>,
}

impl CompletionHooks for Recorder {
    fn pre_call(&mut self, call: PreCall) -> Result<(), PrePostError> {
        self.pre.push(view::pre_value(&call));
        Ok(())
    }

    fn post_call(&mut self, call: PostCall) {
        self.post.push(view::post_value(&call));
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
