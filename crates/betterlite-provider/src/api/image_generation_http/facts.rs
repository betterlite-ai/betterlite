use std::str::from_utf8;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::error::{
    FailureFacts, handle_error, header_text,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageHttpResponse};
use betterlite_openai::error::ProviderError;
use indexmap::IndexMap;
use serde_json::Number;

use super::ClientKind;
use super::error::ImageGenerationError;
use crate::api::image_contracts::generation::BaseImageGenerationConfig;

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) mode: ClientKind,
    pub(super) timeout: Duration,
    pub(super) elapsed: Duration,
}

// httpx's HTTPStatusError hands the handler the decoded body and the response headers it will name, without content-encoding
pub(super) fn status_error<C>(reply: &UpstreamResponse, config: &C) -> ImageGenerationError
where
    C: BaseImageGenerationConfig,
{
    let (text, headers) = masked_reply(reply);
    let facts = FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: None,
        text: Some(text),
        headers: None,
        response: None,
        display: String::new(),
    };
    ImageGenerationError::Provider(Box::new(classify(&facts, &headers, config)))
}

fn masked_reply(reply: &UpstreamResponse) -> (String, ImageHeaders) {
    let text = reply_text(reply);
    let mut headers: ImageHeaders = reply
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect();
    headers.remove("content-encoding");
    headers.remove("content-length");
    if !reply.body.is_empty() {
        let length = reply.body.len().to_string();
        headers.insert(String::from("content-length"), length);
    }
    (text, headers)
}

fn reply_text(reply: &UpstreamResponse) -> String {
    from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    )
}

pub(super) fn transport_error<C>(
    error: &TransportError,
    sent: &Sent,
    config: &C,
) -> ImageGenerationError
where
    C: BaseImageGenerationConfig,
{
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        let facts = FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: None,
            headers: None,
            response: None,
            display: timeout_message(sent),
        };
        return ImageGenerationError::Provider(Box::new(classify(
            &facts,
            &ImageHeaders::new(),
            config,
        )));
    }
    let facts = FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    };
    ImageGenerationError::Provider(Box::new(classify(&facts, &ImageHeaders::new(), config)))
}

fn timeout_message(sent: &Sent) -> String {
    if sent.mode == ClientKind::Sync {
        return format!(
            "litellm.Timeout: Connection timed out after {} seconds.",
            seconds(sent.timeout)
        );
    }
    format!(
        "litellm.Timeout: Connection timed out. Timeout passed={}, time taken={}.{:03} seconds",
        seconds(sent.timeout),
        sent.elapsed.as_secs(),
        sent.elapsed.subsec_millis(),
    )
}

// Python spells a float of whole seconds as 5.0 and every other value as the short decimal str() gives
fn seconds(timeout: Duration) -> String {
    if timeout.subsec_nanos() == 0 {
        return format!("{}.0", timeout.as_secs());
    }
    timeout.as_secs_f64().to_string()
}

// _handle_error picks the status, text and headers, then the provider config names the error class
fn classify<C>(facts: &FailureFacts, headers: &ImageHeaders, config: &C) -> ProviderError
where
    C: BaseImageGenerationConfig,
{
    let base = handle_error(facts, None);
    let mut error = config.get_error_class(&base.message, base.status_code, headers);
    error.status_code_is_synthesized = base.status_code_is_synthesized;
    error
}

pub(super) fn http_response(
    reply: &UpstreamResponse,
) -> Result<ImageHttpResponse, ImageGenerationError> {
    Ok(ImageHttpResponse {
        status_code: reply.status,
        headers: text_headers(&reply.headers)?,
        body: reply.body.clone(),
    })
}

fn text_headers(headers: &http::HeaderMap) -> Result<ImageHeaders, ImageGenerationError> {
    let mut text = ImageHeaders::new();
    for (name, value) in headers {
        let value = value
            .to_str()
            .map_err(|_source| ImageGenerationError::HeaderText {
                name: name.as_str().to_owned(),
            })?;
        text.insert(name.as_str().to_owned(), value.to_owned());
    }
    Ok(text)
}

pub(super) fn provider_headers(headers: &ImageHeaders) -> ProviderHeaderProjection {
    let ordered: IndexMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    set_provider_response_headers(&AdditionalHeaders::new(), &ordered)
}
