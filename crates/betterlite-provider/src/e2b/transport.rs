use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use futures_util::StreamExt as _;

use super::{E2bError, SandboxHttp, SandboxRequest, SandboxStream};

pub struct ProxiedSandboxHttp {
    pub client: ProxiedClient,
}

impl ProxiedSandboxHttp {
    fn outbound(&self, request: SandboxRequest) -> OutboundRequest {
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Json(request.body)
        };
        OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body,
            deadline: self.client.limits.read_timeout,
        }
    }
}

#[async_trait]
impl SandboxHttp for ProxiedSandboxHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, E2bError> {
        let response = self
            .client
            .send(self.outbound(request))
            .await
            .map_err(|source| E2bError::Transport { source })?;
        let status = response.response.status().as_u16();
        Ok(SandboxStream {
            status,
            body: response
                .body_stream()
                .map(|item| item.map_err(|source| E2bError::Body { source }))
                .boxed(),
        })
    }
}
