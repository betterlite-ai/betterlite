use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::{Response, StatusCode};

use crate::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, display_facts, handle_error,
};

use super::super::config::ReadConfig;

// The response transform reads the status, the headers and the bytes the transport read
pub(super) fn http_response(reply: &UpstreamResponse) -> Response<Vec<u8>> {
    let mut response = Response::new(reply.body.clone());
    *response.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}

// _handle_error for a caught transport exception: it carries no response, so the provider error is a synthesized 500 holding its text
pub(super) fn transport_error(config: &dyn ReadConfig, error: &TransportError) -> BaseLlmException {
    let mapped = handle_error(&display_facts(error.to_string()), None);
    let mut failure = config.error_class(&mapped.message, mapped.status_code, HeaderMap::new());
    failure.status_code_is_synthesized = mapped.status_code_is_synthesized;
    failure
}
