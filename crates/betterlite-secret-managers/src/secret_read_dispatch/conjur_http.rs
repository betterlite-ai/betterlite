use std::sync::Arc;

use super::http_transport::{
    self, HttpError, HttpExchange, HttpMethod, HttpRequest, HttpResponse, RequestHeader, into_reply,
};
use crate::cyberark::transport::{
    ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod as ConjurMethod,
};

pub struct ConjurHttpTransport {
    pub exchange: Arc<dyn HttpExchange>,
}

fn spec(request: ConjurRequest<'_>) -> HttpRequest {
    let mut headers = Vec::with_capacity(2);
    if let Some(authorization) = request.authorization {
        headers.push((
            "authorization",
            RequestHeader::Secret(authorization.clone()),
        ));
    }
    if let Some(content_type) = request.content_type {
        headers.push((
            "content-type",
            RequestHeader::Plain(content_type.to_owned()),
        ));
    }
    HttpRequest {
        method: method(request.method),
        url: request.url,
        headers,
        body: request.body.cloned(),
        timeout: http_transport::DEFAULT_TIMEOUT,
        follow_redirects: true,
        verify_tls: request.verify_tls,
        certificate: request.client_certificate.cloned(),
        max_response_bytes: http_transport::MAX_RESPONSE_BYTES,
    }
}

const fn method(method: ConjurMethod) -> HttpMethod {
    match method {
        ConjurMethod::Get => HttpMethod::Get,
        ConjurMethod::Post => HttpMethod::Post,
    }
}

fn conjur_response(response: HttpResponse) -> Result<ConjurResponse, HttpError> {
    let reply = into_reply(response, http_transport::MAX_RESPONSE_BYTES)?;
    Ok(ConjurResponse {
        status: reply.status,
        body: reply.body,
    })
}

impl ConjurTransport for ConjurHttpTransport {
    type Error = HttpError;

    async fn send(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, HttpError> {
        let response = self.exchange.send(spec(request)).await?;
        conjur_response(response)
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, HttpError> {
        let response = self.exchange.send_blocking(spec(request))?;
        conjur_response(response)
    }
}
