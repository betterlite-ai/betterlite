use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, StreamDefaults,
};
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;

use crate::openai_dispatch::logging::{LoggedCall, OpenAiLogger};

// The logging both main.py and the text transport reach, kept as records a case can read back.
#[derive(Default)]
pub struct FixtureLogger {
    pub calls: Vec<LoggedCall>,
    pub pre_calls: Vec<PreCallRecord>,
    pub post_calls: Vec<PostCallRecord>,
}

impl CompletionLogger for FixtureLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre_calls.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post_calls.push(record);
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

impl OpenAiLogger for FixtureLogger {
    fn post_call(&mut self, record: LoggedCall) {
        self.calls.push(record);
    }

    fn completion(&mut self) -> &mut dyn CompletionLogger {
        self
    }
}
