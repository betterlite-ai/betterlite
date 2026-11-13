use std::future::Future;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::copilot::device_auth::transport::{HttpTransport, Reply, TransportFailure};
use crate::openai_base_config::Headers;

pub struct Get<'get> {
    pub url: &'get str,
    pub headers: Headers,
}

pub trait ApiKeyTransport {
    fn get<'call>(
        &'call mut self,
        get: Get<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call;
}

impl ApiKeyTransport for HttpTransport {
    async fn get(&mut self, get: Get<'_>) -> Result<Reply, TransportFailure> {
        let url = Url::parse(get.url).map_err(TransportFailure::Url)?;
        let mut headers = HeaderMap::new();
        for (name, value) in get.headers {
            let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
                TransportFailure::HeaderName {
                    name: name.clone(),
                    source,
                }
            })?;
            let header_value = HeaderValue::try_from(value)
                .map_err(|source| TransportFailure::HeaderValue { name, source })?;
            headers.insert(header_name, header_value);
        }
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.deadline,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(TransportFailure::Request)?;
        Ok(Reply {
            status: response.status,
            body: response.body,
        })
    }
}

mod tests;
