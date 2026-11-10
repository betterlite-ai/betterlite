use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::openai_mock_transport::{
    TransportChoice, handle_mock_request, select_transport,
};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use futures_util::StreamExt as _;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use url::Url;
use uuid::Uuid;

use super::sdk::{
    ByteStream, ClientConstructor, ClientKind, HttpClientSource, RawCompletion, SdkArguments,
    SdkError, TextCompletionSdk,
};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

// Where a constructed SDK client sends: betterlite's pooled outbound client, or the configured in-process mock
#[derive(Clone)]
pub enum ClientTransport {
    Pooled(ProxiedClient),
    Mock {
        now: fn() -> i64,
        next_id: fn() -> Uuid,
    },
}

// The OpenAI SDK over betterlite's shared outbound client; max_retries is accepted but the pool never retries
#[derive(Clone)]
pub struct HttpTextCompletionClient {
    pub transport: ClientTransport,
    pub kind: ClientKind,
    pub api_key: SecretString,
    pub base_url: String,
    pub organization: Option<String>,
    pub timeout: Duration,
}

// Builds a fresh client per call from the call's own settings, so no credential outlives or crosses a call
#[derive(Clone)]
pub struct HttpConstructor {
    pub client_session: Option<ProxiedClient>,
    pub async_http_client: Option<ProxiedClient>,
    pub aclient_session: Option<ProxiedClient>,
    pub fallback: ProxiedClient,
    pub network_mock: bool,
    pub mock_now: fn() -> i64,
    pub mock_next_id: fn() -> Uuid,
}

impl HttpConstructor {
    // litellm.client_session and _get_async_http_client take a supplied client before network_mock; aclient_session is direct.
    fn transport_for(&self, source: HttpClientSource) -> ClientTransport {
        match source {
            HttpClientSource::ClientSession => self.selected(self.client_session.as_ref()),
            HttpClientSource::AsyncHttpClient => self.selected(self.async_http_client.as_ref()),
            HttpClientSource::AClientSession => self.aclient_session.as_ref().map_or_else(
                || ClientTransport::Pooled(self.fallback.clone()),
                |client| ClientTransport::Pooled(client.clone()),
            ),
        }
    }

    fn selected(&self, supplied: Option<&ProxiedClient>) -> ClientTransport {
        match select_transport(supplied.cloned(), self.network_mock, None::<()>) {
            TransportChoice::Supplied(client) => ClientTransport::Pooled(client),
            TransportChoice::Mock => ClientTransport::Mock {
                now: self.mock_now,
                next_id: self.mock_next_id,
            },
            TransportChoice::Default => ClientTransport::Pooled(self.fallback.clone()),
        }
    }
}

impl ClientConstructor for HttpConstructor {
    type Client = HttpTextCompletionClient;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<HttpTextCompletionClient>, SdkError> {
        // an empty key is no key: the SDK raises before it sends anything
        let configured = arguments
            .api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty());
        let Some(api_key) = configured else {
            return Ok(None);
        };
        let timeout =
            Duration::try_from_secs_f64(arguments.timeout).map_err(|_source| SdkError::Other {
                message: format!(
                    "the timeout must be a finite, non-negative number of seconds, not {}",
                    arguments.timeout
                ),
            })?;
        Ok(Some(HttpTextCompletionClient {
            transport: self.transport_for(arguments.http_client),
            kind,
            api_key,
            base_url: arguments
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            organization: arguments.organization.clone(),
            timeout,
        }))
    }
}

fn unreachable_provider(source: &TransportError) -> SdkError {
    SdkError::Other {
        message: format!("the provider could not be reached: {source}"),
    }
}

fn stream_failure(source: &HttpError) -> SdkError {
    SdkError::Other {
        message: format!("the provider stream could not be read: {source}"),
    }
}

fn json_bytes<T>(value: &T) -> Result<Vec<u8>, SdkError>
where
    T: serde::Serialize,
{
    serde_json::to_vec(value).map_err(|source| SdkError::Other {
        message: format!("the request could not be written as JSON: {source}"),
    })
}

fn extra_headers(data: &Map<String, Value>) -> Result<Vec<(String, String)>, SdkError> {
    let Some(Value::Object(fields)) = data.get("extra_headers") else {
        return Ok(Vec::new());
    };
    let mut headers = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        let Some(text) = value.as_str() else {
            return Err(SdkError::Other {
                message: format!("the extra header {name} must be a string"),
            });
        };
        headers.push((name.clone(), text.to_owned()));
    }
    Ok(headers)
}

fn header_value(name: &str, value: &str) -> Result<HeaderValue, SdkError> {
    HeaderValue::try_from(value).map_err(|source| SdkError::Other {
        message: format!("{name} is not a valid header value: {source}"),
    })
}

fn streamed_body(answer: StreamResponse) -> ByteStream {
    Box::pin(
        answer
            .body_stream()
            .map(|chunk| chunk.map_err(|source| stream_failure(&source))),
    )
}

fn mock_answer(
    body: &[u8],
    now: fn() -> i64,
    next_id: fn() -> Uuid,
) -> Result<RawCompletion, SdkError> {
    let answer = handle_mock_request(body, &now, &next_id).map_err(|error| SdkError::Other {
        message: error.to_string(),
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        header_value("content-type", &answer.content_type)?,
    );
    Ok(RawCompletion::Buffered {
        status: answer.status,
        headers,
        body: json_bytes(&answer.body)?,
    })
}

impl HttpTextCompletionClient {
    fn url(&self) -> Result<Url, SdkError> {
        let base = self.base_url.trim_end_matches('/');
        Url::parse(&format!("{base}/completions")).map_err(|source| SdkError::Other {
            message: format!("the base URL is not usable: {source}"),
        })
    }

    fn headers(&self, extra: &[(String, String)]) -> Result<HeaderMap, SdkError> {
        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {}", self.api_key.expose_secret());
        headers.insert(AUTHORIZATION, header_value("authorization", &bearer)?);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(organization) = &self.organization {
            headers.insert(
                HeaderName::from_static("openai-organization"),
                header_value("openai-organization", organization)?,
            );
        }
        for (name, value) in extra {
            let header_name =
                HeaderName::try_from(name.as_str()).map_err(|source| SdkError::Other {
                    message: format!("{name} is not a valid header name: {source}"),
                })?;
            headers.insert(header_name, header_value(name, value)?);
        }
        Ok(headers)
    }

    async fn pooled_answer(
        &self,
        client: &ProxiedClient,
        request: OutboundRequest,
        streaming: bool,
    ) -> Result<RawCompletion, SdkError> {
        if !streaming {
            let answer = client
                .send_buffered(request)
                .await
                .map_err(|source| unreachable_provider(&source))?;
            return Ok(RawCompletion::Buffered {
                status: answer.status,
                headers: answer.headers,
                body: answer.body,
            });
        }
        let answer = client
            .send_following(request)
            .await
            .map_err(|source| unreachable_provider(&source))?;
        let status = answer.response.status().as_u16();
        if status >= 400 {
            // parse() reads the error body before it raises, so the status error is buffered here too
            let failure = answer
                .buffered(client.limits.body_bytes)
                .await
                .map_err(|source| unreachable_provider(&source))?;
            return Ok(RawCompletion::Buffered {
                status: failure.status,
                headers: failure.headers,
                body: failure.body,
            });
        }
        Ok(RawCompletion::Stream {
            status,
            headers: answer.response.headers().clone(),
            body: streamed_body(answer),
        })
    }
}

impl TextCompletionSdk for HttpTextCompletionClient {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    async fn create(&self, data: &Map<String, Value>) -> Result<RawCompletion, SdkError> {
        let extra = extra_headers(data)?;
        let mut body = data.clone();
        body.remove("extra_headers");
        let encoded = json_bytes(&body)?;
        let streaming = matches!(data.get("stream"), Some(Value::Bool(true)));
        match &self.transport {
            ClientTransport::Mock { now, next_id } => mock_answer(&encoded, *now, *next_id),
            ClientTransport::Pooled(client) => {
                let request = OutboundRequest {
                    method: Method::POST,
                    url: self.url()?,
                    query: Vec::new(),
                    headers: self.headers(&extra)?,
                    body: RequestBody::Json(encoded),
                    deadline: self.timeout,
                };
                self.pooled_answer(client, request, streaming).await
            }
        }
    }
}
