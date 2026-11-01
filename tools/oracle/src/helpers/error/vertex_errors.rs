use betterlite_error::facts::ProviderErrorFacts;
use betterlite_error::vertex_errors::map_vertex_exception;
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let facts = ProviderErrorFacts {
            status_code: input.status_code,
            status_code_is_synthesized: false,
            message: None,
            body: None,
            headers: None,
            response: None,
            litellm_response_headers: None,
            request: None,
        };
        let mapped = map_vertex_exception(
            &facts,
            &input.error_str,
            &input.custom_llm_provider,
            input.model.clone(),
        );
        Ok(serde_json::json!({"result": mapped}))
    }
}
