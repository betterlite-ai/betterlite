use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::jina_infinity_rerank::{Raw, Settings};
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::cohere_rerank::RerankParams;
use betterlite_openai::host_rerank::error::HostRerankError;
use betterlite_openai::host_rerank::{
    HostProvider, decode_host_rerank, infinity_complete_url, infinity_request_headers,
    jina_complete_url, jina_request_headers, map_params, transform_request,
};
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

// LiteLLM's uuid4 is replaced by this value on the other side
const FALLBACK_ID: u128 = 0x0000_0000_0000_4000_8000_0000_0000_0001;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: HostProvider,
    model: String,
    options: RerankParams,
    #[serde(default)]
    non_default: RerankParams,
    #[serde(default)]
    headers: Headers,
    #[serde(default)]
    env: BTreeMap<String, String>,
    global_key: Option<String>,
    api_base: Option<String>,
    api_key: Option<String>,
    reply: Option<Reply>,
}

#[derive(Deserialize)]
struct Reply {
    status: u16,
    body: String,
}

pub(crate) struct Op;

// only Infinity carries the upstream status, which is the one error field the two sides compare
fn failure(provider: HostProvider, error: &HostRerankError) -> Box<HelperError> {
    let status = match (provider, error) {
        (HostProvider::Infinity, HostRerankError::Json { status_code, .. }) => Some(*status_code),
        _ => None,
    };
    Box::new(HelperError {
        class: "HostRerankError".to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

fn settings(input: &Input) -> Result<Settings, Box<HelperError>> {
    let raw: Raw = serde_json::from_value(json!({
        "litellm_settings": {"infinity_key": input.global_key},
    }))
    .or_value_error()?;
    raw.convert(&input.env).or_value_error()
}

// Jina's hook sees the raw text only after its status check; Infinity never logs
fn decode_reply(
    provider: HostProvider,
    reply: Option<&Reply>,
    hook: &mut Vec<Value>,
) -> Result<Option<RerankResponse>, Box<HelperError>> {
    let decoded = reply.map(|answer| {
        decode_host_rerank(
            provider,
            answer.status,
            answer.body.as_bytes(),
            || Uuid::from_u128(FALLBACK_ID),
            |text| hook.push(json!(text)),
        )
    });
    decoded
        .transpose()
        .map_err(|error| failure(provider, &error))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let provider = input.provider;
        let key = input.api_key.clone().map(SecretString::from);
        let mapped = map_params(provider, &input.options, &input.non_default);
        let fail = |error: HostRerankError| failure(provider, &error);
        let headers = match provider {
            HostProvider::Jina => jina_request_headers(key.as_ref()),
            HostProvider::Infinity => Ok(infinity_request_headers(
                &input.headers,
                key.as_ref(),
                &settings(input)?,
            )),
        }
        .map_err(fail)?;
        let url = match provider {
            HostProvider::Jina => jina_complete_url(input.api_base.as_deref()),
            HostProvider::Infinity => infinity_complete_url(input.api_base.as_deref()),
        }
        .map_err(fail)?;
        let request = transform_request(provider, &input.model, &mapped).map_err(fail)?;
        let mut hook = Vec::new();
        let response = decode_reply(provider, input.reply.as_ref(), &mut hook)?;
        Ok(json!({
            "mapped": mapped,
            "headers": headers,
            "url": url,
            "request": request,
            "response": response,
            "hook": hook,
        }))
    }
}
