use std::collections::BTreeMap;
use std::fs;
use std::io::{Error as IoError, Read as _};
use std::str::{FromStr as _, Utf8Error, from_utf8};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt as _;
use reqwest::blocking::{
    Client as BlockingClient, ClientBuilder as BlockingClientBuilder, Response as BlockingResponse,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Client, ClientBuilder, Identity, Method};
use secrecy::SecretString;
use tokio::runtime::Handle;

use crate::cyberark::transport::ClientCertificate;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("could not build the HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("could not read the client certificate")]
    CertificateRead(#[source] IoError),
    #[error("the client certificate is not usable")]
    Certificate(#[source] reqwest::Error),
    #[error("the request header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("the request header value is not valid")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("the HTTP request timed out")]
    Timeout,
    #[error("the HTTP request failed")]
    Transport(#[source] reqwest::Error),
    #[error("the HTTP response body could not be read")]
    Body(#[source] IoError),
    #[error("the HTTP response is longer than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the HTTP response body is not valid UTF-8")]
    BodyNotUtf8(#[source] Utf8Error),
    #[error("a blocking HTTP request cannot run inside an async runtime")]
    BlockingInAsyncRuntime,
}

// No Debug: the header values and the body can hold a secret
pub(super) struct Request {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) headers: Vec<(&'static str, String)>,
    pub(super) body: Option<String>,
    pub(super) timeout: Duration,
    pub(super) follow_redirects: bool,
    pub(super) verify_tls: bool,
    pub(super) certificate: Option<ClientCertificate>,
    pub(super) max_response_bytes: usize,
}

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: SecretString,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClientKey {
    follow_redirects: bool,
    verify_tls: bool,
    certificate: Option<(String, String)>,
}

impl ClientKey {
    fn of(request: &Request) -> Self {
        Self {
            follow_redirects: request.follow_redirects,
            verify_tls: request.verify_tls,
            certificate: request.certificate.as_ref().map(|certificate| {
                (
                    certificate.certificate_path.clone(),
                    certificate.key_path.clone(),
                )
            }),
        }
    }
}

struct Clients<T> {
    clients: Mutex<BTreeMap<ClientKey, T>>,
}

impl<T> Default for Clients<T> {
    fn default() -> Self {
        Self {
            clients: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<T: Clone> Clients<T> {
    fn get<Build>(&self, key: &ClientKey, build: Build) -> Result<T, HttpError>
    where
        Build: FnOnce(&ClientKey) -> Result<T, HttpError>,
    {
        if let Some(cached) = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
        {
            return Ok(cached.clone());
        }
        let client = build(key)?;
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), client.clone());
        Ok(client)
    }
}

#[derive(Default)]
pub(super) struct AsyncClients(Clients<Client>);

#[derive(Default)]
pub(super) struct BlockingClients(Clients<BlockingClient>);

fn builder(key: &ClientKey) -> Result<ClientBuilder, HttpError> {
    let builder = ClientBuilder::new()
        .redirect(redirect_policy(key.follow_redirects))
        .tls_danger_accept_invalid_certs(!key.verify_tls);
    let Some((certificate_path, key_path)) = &key.certificate else {
        return Ok(builder);
    };
    let mut pem = fs::read(certificate_path).map_err(HttpError::CertificateRead)?;
    pem.extend(fs::read(key_path).map_err(HttpError::CertificateRead)?);
    let identity = Identity::from_pem(&pem).map_err(HttpError::Certificate)?;
    Ok(builder.identity(identity))
}

fn redirect_policy(follow_redirects: bool) -> Policy {
    if follow_redirects {
        Policy::limited(MAX_REDIRECTS)
    } else {
        Policy::none()
    }
}

fn headers(request: &Request) -> Result<HeaderMap, HttpError> {
    let mut headers = HeaderMap::with_capacity(request.headers.len());
    for (name, value) in &request.headers {
        headers.insert(
            HeaderName::from_str(name).map_err(HttpError::HeaderName)?,
            HeaderValue::from_str(value).map_err(HttpError::HeaderValue)?,
        );
    }
    Ok(headers)
}

fn request_error(source: reqwest::Error) -> HttpError {
    if source.is_timeout() {
        HttpError::Timeout
    } else {
        HttpError::Transport(source)
    }
}

pub(super) async fn send(request: Request, clients: &AsyncClients) -> Result<Reply, HttpError> {
    let client = clients.0.get(&ClientKey::of(&request), |client_key| {
        builder(client_key)?.build().map_err(HttpError::Client)
    })?;
    let header_map = headers(&request)?;
    let response = client
        .request(request.method, request.url)
        .timeout(request.timeout)
        .headers(header_map)
        .body(request.body.unwrap_or_default())
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status().as_u16();
    let body = read_streamed_body(response, request.max_response_bytes).await?;
    Ok(Reply { status, body })
}

async fn read_streamed_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<SecretString, HttpError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(HttpError::Transport)?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(HttpError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    from_utf8(&body)
        .map(SecretString::from)
        .map_err(HttpError::BodyNotUtf8)
}

pub(super) fn send_blocking(
    request: Request,
    clients: &BlockingClients,
) -> Result<Reply, HttpError> {
    if Handle::try_current().is_ok() {
        return Err(HttpError::BlockingInAsyncRuntime);
    }
    let key = ClientKey::of(&request);
    let client = clients.0.get(&key, |client_key| {
        BlockingClientBuilder::from(builder(client_key)?)
            .build()
            .map_err(HttpError::Client)
    })?;
    let limit = request.max_response_bytes;
    let header_map = headers(&request)?;
    let mut request_builder = client
        .request(request.method, request.url)
        .timeout(request.timeout)
        .headers(header_map);
    if let Some(body) = request.body {
        request_builder = request_builder.body(body);
    }
    let response = request_builder.send().map_err(request_error)?;
    let status = response.status().as_u16();
    let body = read_blocking_body(response, limit)?;
    Ok(Reply { status, body })
}

fn read_blocking_body(response: BlockingResponse, limit: usize) -> Result<SecretString, HttpError> {
    let take = u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX);
    let mut body = Vec::new();
    response
        .take(take)
        .read_to_end(&mut body)
        .map_err(HttpError::Body)?;
    if body.len() > limit {
        return Err(HttpError::ResponseTooLarge { limit });
    }
    from_utf8(&body)
        .map(SecretString::from)
        .map_err(HttpError::BodyNotUtf8)
}
