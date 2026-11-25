use betterlite_error::facts::{ProviderErrorFacts, ResponseFacts};
use betterlite_error::openai_like::map_openai_like_exception;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: Option<String>,
    custom_llm_provider: String,
    error_str: String,
    status_code: Option<u16>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let facts = ProviderErrorFacts {
            status_code: input.status_code,
            status_code_is_synthesized: false,
            message: None,
            body: None,
            headers: None,
            response: input.status_code.map(ResponseFacts::base_llm_exception),
            litellm_response_headers: None,
            request: None,
        };
        let mapped = map_openai_like_exception(
            &facts,
            &input.error_str,
            &input.custom_llm_provider,
            input.model.clone(),
        );
        Ok(serde_json::json!({"result": mapped}))
    }
}
