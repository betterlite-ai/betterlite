use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use secrecy::ExposeSecret as _;

use super::error::NomaV2Error;
use super::guardrail::NomaV2Guardrail;
use super::{AUTHORIZATION_PREFIX, SCAN_ENDPOINT};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) async fn call(
    guardrail: &NomaV2Guardrail,
    body: &BTreeMap<String, GuardrailValue>,
) -> Result<GuardrailValue, NomaV2Error> {
    let endpoint = endpoint(&guardrail.settings.api_base)?;
    let mut request = guardrail
        .client
        .post(endpoint)
        .header(CONTENT_TYPE, "application/json");
    if let Some(key) = &guardrail.settings.api_key {
        let authorization = format!("{AUTHORIZATION_PREFIX}{}", key.expose_secret());
        let header = HeaderValue::try_from(authorization.as_str())
            .map_err(|source| NomaV2Error::Header { source })?;
        request = request.header(AUTHORIZATION, header);
    }
    let mut response = request
        .timeout(guardrail.settings.request_timeout)
        .json(body)
        .send()
        .await
        .map_err(transport_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(NomaV2Error::HttpStatus { status });
    }
    let bytes = read_bounded(&mut response, MAX_RESPONSE_BYTES)
        .await
        .map_err(|error| match error {
            ReadError::TooLarge { limit } => NomaV2Error::ResponseTooLarge { limit },
            ReadError::Failed(source) => transport_error(source),
        })?;
    if let Ok(text) = from_utf8(&bytes) {
        tracing::debug!("Noma v2 AIDR response: status_code={status} body={text}");
    }
    serde_json::from_slice(&bytes).map_err(|source| NomaV2Error::ResponseJson { source })
}

fn endpoint(api_base: &str) -> Result<url::Url, NomaV2Error> {
    let text = format!("{api_base}{SCAN_ENDPOINT}");
    url::Url::parse(&text).map_err(|source| NomaV2Error::Endpoint {
        api_base: api_base.to_owned(),
        source,
    })
}

fn transport_error(source: reqwest::Error) -> NomaV2Error {
    if source.is_timeout() {
        return NomaV2Error::RequestTimeout {
            source: source.without_url(),
        };
    }
    NomaV2Error::Transport {
        source: source.without_url(),
    }
}
