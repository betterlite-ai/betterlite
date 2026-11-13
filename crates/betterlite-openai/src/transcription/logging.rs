use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The pre_call LiteLLM's audio_transcriptions makes: no source input, the client's base URL and the data dict.
pub struct TranscriptionPreCall<'record> {
    pub api_base: &'record str,
    pub data: IndexMap<String, Json>,
}

pub struct TranscriptionPostCall<'record> {
    pub input: &'record str,
    pub original_response: Json,
    // the data reaches post_call on success only; the async failure logs the error alone
    pub data: Option<IndexMap<String, Json>>,
}

pub trait TranscriptionLogger {
    fn pre_call(&mut self, record: TranscriptionPreCall<'_>) -> Result<(), PrePostError>;

    fn response_headers(&mut self, headers: &IndexMap<String, String>);

    fn post_call(&mut self, record: TranscriptionPostCall<'_>);
}

pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl TranscriptionLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Null,
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    headers: None,
                    complete_input_dict: Some(RawBody::Object(record.data)),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::from([(
                        String::from("atranscription"),
                        Json::Bool(true),
                    )]),
                },
            },
            self.env,
        )
        .map(drop)
    }

    // LiteLLM keeps the raw response headers on the logging object for B206's projection.
    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        let fields = headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        self.state
            .model_call_details
            .insert(String::from("response_headers"), Json::Object(fields));
    }

    fn post_call(&mut self, record: TranscriptionPostCall<'_>) {
        post_call(
            self.state,
            &PostCall {
                input: Json::Text(record.input.to_owned()),
                original_response: record.original_response,
                additional_args: AdditionalArgs {
                    api_base: None,
                    headers: None,
                    complete_input_dict: record.data.map(RawBody::Object),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        );
    }
}
