use std::sync::Arc;

use super::http_transport::{
    HttpError, HttpExchange, HttpMethod, HttpRequest, HttpResponse, RequestHeader, TlsVerification,
    into_reply,
};
use crate::vault::transport::{VaultMethod, VaultRequest, VaultResponse, VaultTransport};

pub struct VaultHttpTransport {
    pub exchange: Arc<dyn HttpExchange>,
    pub tls: TlsVerification,
}

impl VaultHttpTransport {
    fn call(&self, request: &VaultRequest) -> HttpRequest {
        let mut headers = Vec::with_capacity(3);
        if let Some(token) = request.token.as_ref() {
            headers.push(("x-vault-token", RequestHeader::Secret(token.clone())));
        }
        if let Some(namespace) = request.namespace.as_deref() {
            headers.push((
                "x-vault-namespace",
                RequestHeader::Plain(namespace.to_owned()),
            ));
        }
        if request.body.is_some() {
            headers.push((
                "content-type",
                RequestHeader::Plain("application/json".to_owned()),
            ));
        }
        HttpRequest {
            method: method(request.method),
            url: request.url.clone(),
            headers,
            body: request.body.clone(),
            timeout: request.timeout,
            follow_redirects: request.follow_redirects,
            tls: self.tls.clone(),
            certificate: request.client_certificate.clone(),
            max_response_bytes: request.max_response_bytes,
        }
    }
}

const fn method(method: VaultMethod) -> HttpMethod {
    match method {
        VaultMethod::Get => HttpMethod::Get,
        VaultMethod::Post => HttpMethod::Post,
        VaultMethod::Delete => HttpMethod::Delete,
    }
}

fn vault_response(response: HttpResponse, limit: usize) -> Result<VaultResponse, HttpError> {
    let reply = into_reply(response, limit)?;
    Ok(VaultResponse {
        status: reply.status,
        body: reply.body,
    })
}

impl VaultTransport for VaultHttpTransport {
    type Error = HttpError;

    async fn send(&self, request: VaultRequest) -> Result<VaultResponse, HttpError> {
        let limit = request.max_response_bytes;
        let response = self.exchange.send(self.call(&request)).await?;
        vault_response(response, limit)
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, HttpError> {
        let limit = request.max_response_bytes;
        let response = self.exchange.send_blocking(self.call(&request))?;
        vault_response(response, limit)
    }
}
