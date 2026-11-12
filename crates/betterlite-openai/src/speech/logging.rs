use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The pre_call LiteLLM's audio_speech makes: the source input, the projected body and the client's base URL.
pub struct SpeechLogRecord<'record> {
    pub input: &'record str,
    pub api_base: &'record str,
    pub body: IndexMap<String, Json>,
}

pub trait SpeechLogger {
    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError>;
}

pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl SpeechLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(record.input.to_owned()),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    headers: None,
                    complete_input_dict: Some(RawBody::Object(record.body)),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        )?;
        Ok(())
    }
}
