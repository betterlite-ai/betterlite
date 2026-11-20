use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::stream::flatten_headers;
use http::HeaderMap as HttpHeaders;

use crate::api::vector_store_http_write::error::{BaseLlmException, FailureFacts, handle_error};

use super::super::config::{BufferedResponse, FileContentConfig, Headers};
use super::BufferedReply;

// The response transform reads the status, the joined headers, the bytes the transport read and the URL that answered
pub(super) fn buffered_response(reply: BufferedReply) -> BufferedResponse {
    BufferedResponse {
        status_code: reply.status,
        headers: flatten_headers(&reply.headers).into_iter().collect(),
        body: reply.body,
        url: reply.url.to_string(),
    }
}

// _handle_error for a caught transport exception: it carries no response, so the provider error is a synthesized 500 holding its text
pub(super) fn transport_error(
    config: &dyn FileContentConfig,
    error: &TransportError,
) -> BaseLlmException {
    let facts = FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: error.to_string(),
    };
    let mapped = handle_error(&facts, None);
    let mut failure = config.error_class(&mapped.message, mapped.status_code, Headers::new());
    failure.status_code_is_synthesized = mapped.status_code_is_synthesized;
    failure
}

// The provider's own error class over the raw status, its headers and the body text httpx would print
pub(super) fn status_error(
    config: &dyn FileContentConfig,
    status: u16,
    headers: &HttpHeaders,
    body: &[u8],
) -> BaseLlmException {
    let text = lossy_text(body);
    let joined: Headers = flatten_headers(headers).into_iter().collect();
    config.error_class(&text, status, joined)
}

// httpx's response.text decodes UTF-8 with errors="replace", so each invalid sequence reads as one U+FFFD
pub(super) fn lossy_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
