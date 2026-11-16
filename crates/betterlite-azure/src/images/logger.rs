use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use indexmap::IndexMap;

// The Azure image call sites log B249's hooks with the same fields LiteLLM passes; unlike the OpenAI image
// provider they add no passthrough key, and B249 has no slot for the api_key argument
pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl ImageGenerationLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
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
                    passthrough: IndexMap::new(),
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
