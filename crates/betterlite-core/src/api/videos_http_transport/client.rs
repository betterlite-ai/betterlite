use std::future::Future;
use std::str::from_utf8;
use std::time::{Duration, Instant};

use betterlite_config::tls::TlsPolicyError;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use thiserror::Error;
use url::Url;

use super::error::{SendFailure, VideoTransportError, convert, failed_send};
use super::multipart::boundary;
use super::video_config::{Headers, Params, VideoHttpConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

// What _get_httpx_client and get_async_httpx_client key their cached client on
#[derive(Clone, Copy, Debug)]
pub struct ClientRequest<'request> {
    pub kind: ClientKind,
    pub ssl_verify: Option<bool>,
    // get_async_httpx_client caches per llm_provider, while _get_httpx_client takes no provider at all
    pub custom_llm_provider: &'request str,
}

// httpx's post and get, which differ in what they do with a failing status and with a timeout
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HttpVerb {
    Post,
    Get,
}

impl HttpVerb {
    pub(crate) const fn method(self) -> Method {
        match self {
            Self::Post => Method::POST,
            Self::Get => Method::GET,
        }
    }
}

// What a failed send needs from the call that made it, for the message LiteLLM's post raises on a timeout
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sent {
    pub(crate) mode: ClientKind,
    pub(crate) verb: HttpVerb,
    pub(crate) timeout: Option<Duration>,
}

#[derive(Debug)]
pub(crate) enum ServeBody<'source> {
    Empty,
    Json(&'source Params),
    Parts(Vec<MultipartPart>),
}

// The sync and async HTTP handler classes over one transport: the request a video call sends
pub trait VideoTransport: Send + Sync {
    fn send(
        &self,
        request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

// One built wire request: the URL, the headers and the body httpx would send, with the timeout LiteLLM passed
#[derive(Debug)]
pub struct VideoRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// The collaborators every video transport call shares: the provider config, the client source and a caller-supplied client
pub struct VideoDeps<'deps, C, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub trait ClientSource: Send + Sync {
    type Client: VideoTransport;

    fn client(&self, request: ClientRequest<'_>) -> Result<Self::Client, ClientError>;
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
}

pub(crate) enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    #[must_use]
    pub(crate) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

// A supplied client of the other kind is not the one LiteLLM would use, so the source builds the one asked for
pub(crate) fn select_client<'deps, C, S>(
    deps: &'deps VideoDeps<'deps, C, S>,
    request: ClientRequest<'_>,
) -> Result<Selected<'deps, S::Client>, ClientError>
where
    S: ClientSource,
{
    let kind = request.kind;
    if let Some(supplied) = &deps.supplied
        && supplied.kind == kind
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(request)?))
}

// The request httpx builds before its send: the URL, the headers and the body, each raising inside the handler's try
pub(crate) fn build(
    method: Method,
    url: &str,
    headers: &Headers,
    body: ServeBody<'_>,
    timeout: Option<Duration>,
) -> Result<VideoRequest, SendFailure> {
    let parsed = Url::parse(url).map_err(|source| SendFailure::Other {
        message: format!("the request URL {url} is not a URL: {source}"),
    })?;
    Ok(VideoRequest {
        method,
        url: parsed,
        headers: wire_headers(headers)?,
        body: wire_body(body)?,
        timeout,
    })
}

fn wire_headers(headers: &Headers) -> Result<HeaderMap, SendFailure> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| SendFailure::Other {
                message: format!("the {name} header name is not a header name: {source}"),
            })?;
        let header_value = HeaderValue::from_str(value).map_err(|source| SendFailure::Other {
            message: format!("the {name} header value is not a header value: {source}"),
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn wire_body(body: ServeBody<'_>) -> Result<RequestBody, SendFailure> {
    match body {
        ServeBody::Empty => Ok(RequestBody::Empty),
        ServeBody::Json(data) => {
            let encoded = serde_json::to_vec(data).map_err(|source| SendFailure::Other {
                message: format!("the request body could not be encoded: {source}"),
            })?;
            Ok(RequestBody::Json(encoded))
        }
        ServeBody::Parts(parts) => Ok(RequestBody::Multipart {
            boundary: boundary(&parts),
            parts,
        }),
    }
}

// The try around LiteLLM's send: whatever the transport or the request build raised goes through _handle_error
pub(crate) async fn exchange<C, Client>(
    config: &C,
    client: &Client,
    request: Result<VideoRequest, SendFailure>,
    sent: Sent,
) -> Result<UpstreamResponse, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    Client: VideoTransport,
{
    let outcome = match request {
        Ok(request) => send(client, request, sent).await,
        Err(failure) => Err(failure),
    };
    outcome.map_err(|failure| VideoTransportError::Provider(Box::new(convert(config, failure))))
}

async fn send<Client>(
    client: &Client,
    request: VideoRequest,
    sent: Sent,
) -> Result<UpstreamResponse, SendFailure>
where
    Client: VideoTransport,
{
    let started = Instant::now();
    let reply = client.send(request).await;
    let elapsed = started.elapsed();
    let response = match reply {
        Ok(response) => response,
        Err(error) => return Err(failed_send(&error, sent, elapsed)),
    };
    if sent.verb == HttpVerb::Post && response.status >= 400 {
        return Err(SendFailure::Status {
            status: response.status,
            headers: response_headers(&response),
            text: body_text(&response.body),
        });
    }
    Ok(response)
}

// The masked HTTPStatusError keeps every response header but the encoding and the length of the compressed body
fn response_headers(response: &UpstreamResponse) -> Headers {
    response
        .headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| {
            let text = value.to_str().map_or_else(
                |_| format!("{} bytes that are not UTF-8", value.as_bytes().len()),
                ToOwned::to_owned,
            );
            (name.as_str().to_owned(), text)
        })
        .collect()
}

fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}

// The pooled client's wire adapter: the request's timeout or the client's own read timeout sets the deadline
impl VideoTransport for ProxiedClient {
    async fn send(&self, request: VideoRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        self.send_following(OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: request.body,
            deadline,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}
