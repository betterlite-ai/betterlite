use std::error::Error as StdError;
use std::str;

use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::types::TraceApi;
use crate::http_analytics_loggers::delivery::{
    DELIVERY_DEADLINE, Delivery, PostRequest, PostResponse,
};

mod tests;

pub const API_BASE_URL: &str = "https://api.confident-ai.com";
pub const TRACING_ENDPOINT: &str = "/v1/tracing";
pub const CONFIDENT_API_KEY_HEADER: &str = "confident_api_key";

pub type Reply<E> = Result<PostResponse, ApiError<E>>;
pub type Ack<E> = Result<(), ApiError<E>>;

// litellm.integrations.deepeval.api.HttpMethods; only POST reaches the wire, the other names stay declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
    Put,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError<E>
where
    E: StdError + 'static,
{
    #[error("only POST requests are supported")]
    Method,
    #[error("the Confident AI base URL could not be parsed")]
    Url(#[source] url::ParseError),
    #[error("the CONFIDENT_API_KEY header could not be built")]
    Header(#[source] InvalidHeaderValue),
    #[error("the trace body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Confident AI API answered with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the Confident AI request failed")]
    Transport(#[source] E),
}

pub struct Api<D> {
    delivery: D,
    endpoint: Url,
    headers: HeaderMap,
}

impl<D: Delivery> Api<D> {
    pub fn new(api_key: &SecretString, delivery: D) -> Result<Self, ApiError<D::Error>> {
        let key: &str = api_key.expose_secret();
        let mut value = HeaderValue::try_from(key).map_err(ApiError::Header)?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(HeaderName::from_static(CONFIDENT_API_KEY_HEADER), value);
        let endpoint =
            Url::parse(&format!("{API_BASE_URL}{TRACING_ENDPOINT}")).map_err(ApiError::Url)?;
        Ok(Self {
            delivery,
            endpoint,
            headers,
        })
    }

    // Api.send_request: _http_request POSTs, then the 200 check decides whether the response comes back.
    pub async fn send_request(&self, method: HttpMethod, trace: &TraceApi) -> Reply<D::Error> {
        let response = self.deliver(method, trace).await?;
        if response.status == 200 {
            return Ok(response);
        }
        Err(status_error(&response))
    }

    // Api.a_send_request: the async handler raises for a non-2xx status and the answer itself is dropped.
    pub async fn a_send_request(&self, method: HttpMethod, trace: &TraceApi) -> Ack<D::Error> {
        let response = self.deliver(method, trace).await?;
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        Err(status_error(&response))
    }

    async fn deliver(&self, method: HttpMethod, trace: &TraceApi) -> Reply<D::Error> {
        if method != HttpMethod::Post {
            return Err(ApiError::Method);
        }
        let body = serde_json::to_vec(trace).map_err(ApiError::Encode)?;
        let request = PostRequest {
            url: self.endpoint.clone(),
            headers: self.headers.clone(),
            body,
            deadline: DELIVERY_DEADLINE,
        };
        self.delivery
            .post(request)
            .await
            .map_err(ApiError::Transport)
    }
}

fn status_error<E>(response: &PostResponse) -> ApiError<E>
where
    E: StdError + 'static,
{
    ApiError::Status {
        status: response.status,
        body: str::from_utf8(&response.body).map_or_else(|_| String::new(), String::from),
    }
}
