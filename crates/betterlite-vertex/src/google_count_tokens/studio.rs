use std::{collections::BTreeMap, time::Duration};

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::google_count_tokens::transport::{
    CountRequest, CountTransport, count, count_body, header_map,
};
use crate::google_count_tokens::{decode::GoogleCountResponse, error::GoogleCountError};
use crate::studio_identity::{AuthValue, NativeParams, native_headers};

pub struct StudioCountInput<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub params: &'request NativeParams,
    pub settings: &'request Settings,
    // LiteLLM accepts a timeout here and never forwards it; the deadline belongs to the pooled client
    pub timeout: Option<Duration>,
}

// LiteLLM's GoogleAIStudioTokenCounter.validate_environment: native GenAI headers and the
// {api_base-or-default}/v1beta/models/{model}:countTokens URL
pub fn studio_request(input: &StudioCountInput<'_>) -> Result<CountRequest, GoogleCountError> {
    let explicit = input.api_key.cloned().map(AuthValue::Key);
    let headers = native_headers(
        explicit.as_ref(),
        input.params,
        input.settings,
        &BTreeMap::new(),
    );
    let base = input.api_base.unwrap_or(DEFAULT_API_BASE);
    Ok(CountRequest {
        url: format!("{base}/v1beta/models/{}:countTokens", input.model),
        headers: header_map(&headers)?,
        body: count_body(input.contents)?,
    })
}

pub async fn count_tokens<T: CountTransport>(
    transport: &T,
    input: &StudioCountInput<'_>,
    deadline: Duration,
) -> Result<GoogleCountResponse, GoogleCountError> {
    count(transport, studio_request(input)?, deadline).await
}
