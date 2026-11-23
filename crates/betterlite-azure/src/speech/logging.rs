use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
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

pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl SpeechLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: SpeechPreCall<'_>) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(record.input.to_owned()),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    complete_input_dict: Some(RawBody::Object(record.data)),
                    ..AdditionalArgs::default()
                },
            },
            self.env,
        )
        .map(drop)
    }
}
