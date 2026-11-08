use std::sync::Arc;

use secrecy::ExposeSecret as _;

use super::http_transport::{self, HttpError, HttpExchange, HttpMethod, HttpRequest, into_reply};
use crate::cyberark::transport::{
    ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod as ConjurMethod,
};

pub struct ConjurHttpTransport {
    pub exchange: Arc<dyn HttpExchange>,
}

fn spec(request: ConjurRequest<'_>) -> HttpRequest {
    let mut headers = Vec::with_capacity(2);
    if let Some(authorization) = request.authorization {
        headers.push(("authorization", authorization.expose_secret().to_owned()));
    }
    if let Some(content_type) = request.content_type {
        headers.push(("content-type", content_type.to_owned()));
    }
    HttpRequest {
        method: method(request.method),
        url: request.url,
        headers,
        body: request.body.map(|body| body.expose_secret().to_owned()),
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

fn conjur_response(response: http_transport::HttpResponse) -> Result<ConjurResponse, HttpError> {
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
        let answer = into_reply(response, http_transport::MAX_RESPONSE_BYTES)?;
        Ok(ConjurResponse {
            status: answer.status,
            body: answer.body,
        })
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, HttpError> {
        let response = self.exchange.send_blocking(spec(request))?;
        conjur_response(response)
    }
}
