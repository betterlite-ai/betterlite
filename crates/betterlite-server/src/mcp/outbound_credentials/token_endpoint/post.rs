use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
// Betterlite's transport is one error type, so the detail names it where LiteLLM names httpx's class.
const TRANSPORT_KIND: &str = "TransportError";

#[derive(Debug)]
pub enum PostedForm {
    Answered { status: u16, body: Vec<u8> },
    Unreachable { kind: &'static str },
}

#[async_trait]
pub trait TokenEndpointPost: Send + Sync {
    async fn post(&self, endpoint: &str, form: &[(String, SecretString)]) -> PostedForm;
}

pub struct HttpTokenPost {
    pub client: Arc<ProxiedClient>,
    pub deadline: Duration,
}

#[async_trait]
impl TokenEndpointPost for HttpTokenPost {
    async fn post(&self, endpoint: &str, form: &[(String, SecretString)]) -> PostedForm {
        let Ok(url) = Url::parse(endpoint) else {
            return PostedForm::Unreachable { kind: "InvalidUrl" };
        };
        let mut encoded = form_urlencoded::Serializer::new(String::new());
        for (name, value) in form {
            encoded.append_pair(name, value.expose_secret());
        }
        // The serializer borrows a closure that is not Sync, so it is gone before the request awaits.
        let data = encoded.finish().into_bytes();
        drop(encoded);
        let request = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
                data,
            },
            deadline: self.deadline,
        };
        let queued = match self.client.send_following(request).await {
            Ok(queued) => queued,
            Err(_error) => {
                return PostedForm::Unreachable {
                    kind: TRANSPORT_KIND,
                };
            }
        };
        match queued.buffered(self.client.limits.body_bytes).await {
            Ok(response) => PostedForm::Answered {
                status: response.status,
                body: response.body,
            },
            Err(_error) => PostedForm::Unreachable {
                kind: TRANSPORT_KIND,
            },
        }
    }
}
