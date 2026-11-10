use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderValue;

use crate::langfuse_exporter::PROTOBUF_CONTENT_TYPE;
use crate::langfuse_exporter::transport::{HttpRequest, HttpResponse, Transport};

#[derive(Clone, Debug)]
pub struct HttpTransport {
    pub client: ProxiedClient,
}

impl Transport for HttpTransport {
    type Error = TransportError;

    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let outbound = OutboundRequest {
            method: Method::POST,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static(PROTOBUF_CONTENT_TYPE),
                data: request.body.to_vec(),
            },
            deadline: request.timeout,
        };
        let response = self.client.send_buffered(outbound).await?;
        Ok(HttpResponse {
            status: response.status,
        })
    }
}
