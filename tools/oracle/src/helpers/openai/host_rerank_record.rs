use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::jina_infinity_rerank::{Raw, Settings};
use betterlite_openai::cohere_rerank::RerankParams;
use betterlite_openai::host_rerank::error::HostRerankError;
use betterlite_openai::host_rerank::{
    HostProvider, decode_host_rerank, infinity_complete_url, infinity_request_headers,
    jina_complete_url, jina_request_headers, jina_rerank_cost, map_params, transform_request,
};
use betterlite_openai::openai_base_config::Headers;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

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
    input_cost_per_token: Option<f64>,
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

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let provider = input.provider;
        let key = input.api_key.clone().map(SecretString::from);
        let mapped = map_params(provider, &input.options, &input.non_default);
        let (headers, url) = match provider {
            HostProvider::Jina => (
                jina_request_headers(key.as_ref()),
                jina_complete_url(input.api_base.as_deref()),
            ),
            HostProvider::Infinity => (
                Ok(infinity_request_headers(
                    &input.headers,
                    key.as_ref(),
                    &settings(input)?,
                )),
                infinity_complete_url(input.api_base.as_deref()),
            ),
        };
        let fail = |error: HostRerankError| failure(provider, &error);
        let headers = headers.map_err(fail)?;
        let url = url.map_err(fail)?;
        let request = transform_request(provider, &input.model, &mapped).map_err(fail)?;
        let mut hook = Vec::new();
        let response = input
            .reply
            .as_ref()
            .map(|reply| {
                decode_host_rerank(
                    provider,
                    reply.status,
                    reply.body.as_bytes(),
                    || Uuid::from_u128(FALLBACK_ID),
                    |text| hook.push(json!(text)),
                )
            })
            .transpose()
            .map_err(fail)?;
        let cost = match (provider, &response) {
            (HostProvider::Jina, Some(decoded)) => {
                let billed = decoded
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.billed_units.as_ref())
                    .and_then(Option::as_ref);
                let price = input
                    .input_cost_per_token
                    .map(Decimal::try_from)
                    .transpose()
                    .or_value_error()?;
                let (prompt, completion) = jina_rerank_cost(billed, price).map_err(fail)?;
                json!([prompt.to_f64(), completion.to_f64()])
            }
            _ => Value::Null,
        };
        Ok(json!({
            "mapped": mapped,
            "headers": headers,
            "url": url,
            "request": request,
            "response": response,
            "hook": hook,
            "cost": cost,
        }))
    }
}
