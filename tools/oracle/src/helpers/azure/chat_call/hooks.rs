use betterlite_azure::azure_stream_errors::records::PreCallRecord;
use betterlite_azure::chat_call::hooks::ChatHooks;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use indexmap::IndexMap;

// The logging object the handler calls: the oracle records the pre and post calls instead of emitting them
#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<PreCallRecord>,
    pub(super) post: Vec<PostCall>,
    pub(super) response_headers: Vec<IndexMap<String, String>>,
}

impl ChatHooks for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, call: PostCall) {
        self.post.push(call);
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.response_headers.push(headers.clone());
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
