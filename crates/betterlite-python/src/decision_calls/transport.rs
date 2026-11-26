use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_provider::opensandbox::helpers::{
    OpenSandboxError, SandboxHttp, SandboxRequest, SandboxStream,
};
use futures_util::StreamExt as _;

// The OpenSandbox half of the dispatch sends through the session's proxied client, as the e2b half does.
pub(crate) struct ProxiedOpenSandboxHttp {
    pub(crate) client: ProxiedClient,
}

impl ProxiedOpenSandboxHttp {
    fn outbound(&self, request: SandboxRequest) -> OutboundRequest {
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Json(request.body)
        };
        let deadline = request
            .timeout
            .and_then(|timeout| timeout.read)
            .unwrap_or(self.client.limits.read_timeout);
        OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body,
            deadline,
        }
    }
}

#[async_trait]
impl SandboxHttp for ProxiedOpenSandboxHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        let response = self
            .client
            .send(self.outbound(request))
            .await
            .map_err(|source| OpenSandboxError::Transport { source })?;
        let status = response.response.status().as_u16();
        Ok(SandboxStream {
            status,
            body: response
                .body_stream()
                .map(|item| item.map_err(|source| OpenSandboxError::Body { source }))
                .boxed(),
        })
    }
}
