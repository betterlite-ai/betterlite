use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// the source methods' logging_obj: pre_call before the call, post_call for the reply or the failure
pub trait ImageGenerationLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    fn post_call(&mut self, record: PostCallRecord);
}

pub struct PreCallRecord {
    pub input: String,
    pub headers: IndexMap<String, Json>,
    pub api_base: String,
    pub data: IndexMap<String, Json>,
}

pub struct PostCallRecord {
    pub input: String,
    pub original_response: Json,
    // the body reaches post_call on success and on the sync failure only
    pub data: Option<IndexMap<String, Json>>,
}

// The logging crate's pre-call and post-call dispatch over the source methods' records.
pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl ImageGenerationLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let mut passthrough = IndexMap::new();
        passthrough.insert(String::from("acompletion"), Json::Bool(true));
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(record.input),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base),
                    headers: Some(record.headers),
                    complete_input_dict: Some(RawBody::Object(record.data)),
                    request_str: None,
                    openai_sdk: None,
                    passthrough,
                },
            },
            self.env,
        )
        .map(drop)
    }

    fn post_call(&mut self, record: PostCallRecord) {
        post_call(
            self.state,
            &PostCall {
                input: Json::Text(record.input),
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
