use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_error::facts::{ProviderErrorFacts, RequestFacts, ResponseFacts};
use betterlite_error::native_errors_a::ai21::map_ai21_exception;
use betterlite_error::native_errors_a::cloudflare::map_cloudflare_with_status;
use betterlite_error::native_errors_a::cohere::map_cohere_exception;
use betterlite_error::native_errors_a::huggingface::map_huggingface_exception;
use serde::Deserialize;

// mirrors BaseLLMException's own response=httpx.Response(status_code, request=httpx.Request(...))
fn stub_response(status_code: Option<u16>) -> Option<ResponseFacts> {
    let status = status_code?;
    Some(ResponseFacts {
        status_code: status,
        headers: BTreeMap::new(),
        request: Some(RequestFacts {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        }),
    })
}

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
    #[serde(default)]
    status_code: Option<u16>,
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
            Input::Cloudflare(fields) => map_cloudflare_with_status(
                &fields.error_str,
                fields.model.clone(),
                fields.status_code,
            ),
            Input::Cohere(fields) => {
                let facts = ProviderErrorFacts {
                    status_code: fields.status_code,
                    status_code_is_synthesized: false,
                    message: None,
                    body: None,
                    headers: None,
                    response: stub_response(fields.status_code),
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
            Input::Huggingface(fields) => {
                let facts = ProviderErrorFacts {
                    status_code: fields.status_code,
                    status_code_is_synthesized: false,
                    message: None,
                    body: None,
                    headers: None,
                    response: stub_response(fields.status_code),
                    litellm_response_headers: None,
                    request: None,
                };
                map_huggingface_exception(&facts, &fields.error_str, fields.model.clone())
            }
            Input::Ai21(fields) => run_ai21(fields),
        };
        Ok(serde_json::json!({"result": mapped}))
    }
}

fn run_ai21(fields: &Ai21Input) -> Option<ErrorClass> {
    let facts = ProviderErrorFacts {
        status_code: fields.status_code,
        status_code_is_synthesized: false,
        message: fields.message.clone(),
        body: None,
        headers: None,
        response: stub_response(fields.status_code),
        litellm_response_headers: None,
        request: None,
    };
    map_ai21_exception(&facts, fields.model.clone())
}
