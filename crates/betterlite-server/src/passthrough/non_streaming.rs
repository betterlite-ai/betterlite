use betterlite_core::api::request_form::values::FormParts;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use http::Method;
use serde_json::Value;
use thiserror::Error;

use super::multipart::{is_multipart, make_multipart_http_request};
use super::upstream::UpstreamTarget;

#[derive(Debug, Error)]
pub enum NonStreamingError {
    #[error("a multipart request reached the handler without its decoded form")]
    UndecodedForm,
    #[error("the request body could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub struct NonStreamingRequest<'request> {
    pub target: UpstreamTarget,
    pub parsed_body: Option<&'request Value>,
    pub form: Option<&'request FormParts>,
    pub forward_multipart: bool,
    pub boundary: String,
}

// LiteLLM's non_streaming_http_request_handler: GET wins over multipart, and a multipart call is only forwarded when the caller decoded the form
pub async fn non_streaming_request_handler(
    client: &ProxiedClient,
    request: NonStreamingRequest<'_>,
) -> Result<StreamResponse, NonStreamingError> {
    let NonStreamingRequest {
        target,
        parsed_body,
        form,
        forward_multipart,
        boundary,
    } = request;
    let body = if target.method == Method::GET {
        RequestBody::Empty
    } else if is_multipart(&target.headers) && forward_multipart {
        let form = form.ok_or(NonStreamingError::UndecodedForm)?;
        let outbound = make_multipart_http_request(target, form, &boundary);
        return client
            .send(outbound)
            .await
            .map_err(NonStreamingError::Transport);
    } else {
        match parsed_body {
            Some(value) => RequestBody::Json(
                serde_json::to_vec(value).map_err(|source| NonStreamingError::Encode { source })?,
            ),
            None => RequestBody::Empty,
        }
    };
    let outbound = OutboundRequest {
        method: target.method,
        url: target.url,
        query: target.query,
        headers: target.headers,
        body,
        deadline: target.deadline,
    };
    client
        .send(outbound)
        .await
        .map_err(NonStreamingError::Transport)
}
