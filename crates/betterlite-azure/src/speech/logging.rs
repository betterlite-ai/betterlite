use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The pre_call audio_speech makes: the speech input, the client base URL and speech_request_body's dict.
pub struct SpeechPreCall<'record> {
    pub input: &'record str,
    pub api_base: &'record str,
    pub data: IndexMap<String, Json>,
}

pub trait SpeechLogger {
    fn pre_call(&mut self, record: SpeechPreCall<'_>) -> Result<(), PrePostError>;
}

impl SpeechLogger for LogContext<'_, '_> {
    fn pre_call(&mut self, record: SpeechPreCall<'_>) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(record.input.to_owned()),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    headers: None,
                    complete_input_dict: Some(RawBody::Object(record.data)),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        )
        .map(drop)
    }
}
