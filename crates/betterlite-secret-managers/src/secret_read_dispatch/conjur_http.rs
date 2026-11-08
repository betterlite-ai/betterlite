use reqwest::Method;
use secrecy::ExposeSecret as _;

use super::http_transport::{self, AsyncClients, BlockingClients, HttpError, Request};
use crate::cyberark::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};

#[derive(Default)]
pub struct ConjurHttpTransport {
    async_clients: AsyncClients,
    blocking_clients: BlockingClients,
}

fn spec(request: ConjurRequest<'_>) -> Request {
    let mut headers = Vec::with_capacity(2);
    if let Some(authorization) = request.authorization {
        headers.push(("authorization", authorization.expose_secret().to_owned()));
    }
    if let Some(content_type) = request.content_type {
        headers.push(("content-type", content_type.to_owned()));
    }
    Request {
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

const fn method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
    }
}

fn conjur_response(reply: http_transport::Reply) -> ConjurResponse {
    ConjurResponse {
        status: reply.status,
        body: reply.body,
    }
}

impl ConjurTransport for ConjurHttpTransport {
    type Error = HttpError;

    async fn send(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, HttpError> {
        let reply = http_transport::send(spec(request), &self.async_clients).await?;
        Ok(conjur_response(reply))
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, HttpError> {
        let reply = http_transport::send_blocking(spec(request), &self.blocking_clients)?;
        Ok(conjur_response(reply))
    }
}
