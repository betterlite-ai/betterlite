use reqwest::Method;
use secrecy::ExposeSecret as _;

use super::http_transport::{self, AsyncClients, BlockingClients, HttpError, Request};
use crate::vault::transport::{VaultMethod, VaultRequest, VaultResponse, VaultTransport};

#[derive(Default)]
pub struct VaultHttpTransport {
    async_clients: AsyncClients,
    blocking_clients: BlockingClients,
}

impl VaultHttpTransport {
    fn call(request: &VaultRequest) -> Request {
        let mut headers = Vec::with_capacity(3);
        if let Some(token) = request.token.as_ref() {
            headers.push(("x-vault-token", token.expose_secret().to_owned()));
        }
        if let Some(namespace) = request.namespace.as_deref() {
            headers.push(("x-vault-namespace", namespace.to_owned()));
        }
        if request.body.is_some() {
            headers.push(("content-type", "application/json".to_owned()));
        }
        Request {
            method: method(request.method),
            url: request.url.clone(),
            headers,
            body: request
                .body
                .as_ref()
                .map(|body| body.expose_secret().to_owned()),
            timeout: request.timeout,
            follow_redirects: request.follow_redirects,
            verify_tls: true,
            certificate: request.client_certificate.clone(),
            max_response_bytes: request.max_response_bytes,
        }
    }
}

const fn method(method: VaultMethod) -> Method {
    match method {
        VaultMethod::Get => Method::GET,
        VaultMethod::Post => Method::POST,
        VaultMethod::Delete => Method::DELETE,
    }
}

fn vault_response(reply: http_transport::Reply) -> VaultResponse {
    VaultResponse {
        status: reply.status,
        body: reply.body,
    }
}

impl VaultTransport for VaultHttpTransport {
    type Error = HttpError;

    async fn send(&self, request: VaultRequest) -> Result<VaultResponse, HttpError> {
        let reply = http_transport::send(Self::call(&request), &self.async_clients).await?;
        Ok(vault_response(reply))
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, HttpError> {
        let reply = http_transport::send_blocking(Self::call(&request), &self.blocking_clients)?;
        Ok(vault_response(reply))
    }
}
