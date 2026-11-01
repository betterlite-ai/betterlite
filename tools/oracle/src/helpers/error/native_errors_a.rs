use betterlite_error::error::ErrorClass;
use betterlite_error::facts::ProviderErrorFacts;
use betterlite_error::native_errors_a::ai21::map_ai21_exception;
use betterlite_error::native_errors_a::cloudflare::map_cloudflare_exception;
use betterlite_error::native_errors_a::cohere::map_cohere_exception;
use betterlite_error::native_errors_a::huggingface::map_huggingface_exception;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Cloudflare(CloudflareInput),
    Cohere(CohereInput),
    Huggingface(HuggingfaceInput),
    Ai21(Ai21Input),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloudflareInput {
    model: Option<String>,
    error_str: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohereInput {
    model: Option<String>,
    error_str: String,
    exception_type: String,
    status_code: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HuggingfaceInput {
    model: Option<String>,
    error_str: String,
    status_code: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ai21Input {
    model: Option<String>,
    message: Option<String>,
    status_code: Option<u16>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mapped = match input {
            Input::Cloudflare(fields) => {
                map_cloudflare_exception(&fields.error_str, fields.model.clone())
            }
            Input::Cohere(fields) => run_cohere(fields),
            Input::Huggingface(fields) => run_huggingface(fields),
            Input::Ai21(fields) => run_ai21(fields),
        };
        Ok(serde_json::json!({"result": mapped}))
    }
}

fn run_cohere(fields: &CohereInput) -> Option<ErrorClass> {
    let facts = ProviderErrorFacts {
        status_code: fields.status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    map_cohere_exception(
        &facts,
        &fields.error_str,
        &fields.exception_type,
        fields.model.clone(),
    )
}

fn run_huggingface(fields: &HuggingfaceInput) -> Option<ErrorClass> {
    let facts = ProviderErrorFacts {
        status_code: fields.status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    map_huggingface_exception(&facts, &fields.error_str, fields.model.clone())
}

fn run_ai21(fields: &Ai21Input) -> Option<ErrorClass> {
    let facts = ProviderErrorFacts {
        status_code: fields.status_code,
        status_code_is_synthesized: false,
        message: fields.message.clone(),
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    map_ai21_exception(&facts, fields.model.clone())
}
