use betterlite_azure::azure_stream_errors::records::{PreCallRecord, StreamHooks};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use indexmap::IndexMap;

// The logging object the streaming paths call: the oracle records the pre call and the stored headers
#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<PreCallRecord>,
    pub(super) response_headers: Option<IndexMap<String, String>>,
}

impl StreamHooks for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.response_headers = Some(headers.clone());
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
