use std::collections::BTreeMap;

use betterlite_error::facts::{ProviderErrorFacts, RequestFacts, ResponseFacts};
use betterlite_error::openai_like::map_openai_like_exception;
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
            response: stub_response(input.status_code),
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
