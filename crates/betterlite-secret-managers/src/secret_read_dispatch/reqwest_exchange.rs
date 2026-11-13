use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};

use reqwest::blocking::{
    Client as BlockingClient, ClientBuilder as BlockingClientBuilder, Response as BlockingResponse,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Certificate, Client, ClientBuilder, Identity, Method};
use secrecy::ExposeSecret as _;
use tokio::fs as async_fs;
use tokio::runtime::Handle;

use super::http_transport::{
    ExchangeFuture, HttpError, HttpExchange, HttpMethod, HttpRequest, HttpResponse, TlsVerification,
};

const MAX_REDIRECTS: usize = 20;

#[derive(Default)]
pub struct ReqwestExchange {
    async_clients: Clients<Client>,
    blocking_clients: Clients<BlockingClient>,
}

impl ReqwestExchange {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let client_key = ClientKey::of(&request);
        let client = if let Some(cached) = self.async_clients.cached(&client_key) {
            cached
        } else {
            let built = async_builder(&client_key)
                .await?
                .build()
                .map_err(HttpError::Client)?;
            self.async_clients.store(&client_key, built.clone());
            built
        };
        let mut outgoing = client
            .request(method(request.method), &request.url)
            .timeout(request.timeout)
            .headers(headers(&request)?);
        if let Some(body) = &request.body {
            outgoing = outgoing.body(body.expose_secret().to_owned());
        }
        let response = outgoing.send().await.map_err(request_error)?;
        let status = response.status().as_u16();
        read_streamed_body(response, request.max_response_bytes)
            .await
            .map(|body| HttpResponse { status, body })
    }

    fn execute_blocking(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let client = self.blocking_clients.handle(request)?;
        let response = blocking_request(&client, request)?;
        let status = response.status().as_u16();
        let body = read_blocking_body(response, request.max_response_bytes)?;
        Ok(HttpResponse { status, body })
    }
}

fn blocking_request(
    client: &BlockingClient,
    request: &HttpRequest,
) -> Result<BlockingResponse, HttpError> {
    client
        .request(method(request.method), &request.url)
        .timeout(request.timeout)
        .headers(headers(request)?)
        .body(payload(request))
        .send()
        .map_err(request_error)
}

fn payload(request: &HttpRequest) -> String {
    request
        .body
        .as_ref()
        .map_or_else(String::new, |body| body.expose_secret().to_owned())
}

impl HttpExchange for ReqwestExchange {
    fn send(&self, request: HttpRequest) -> ExchangeFuture<'_> {
        Box::pin(self.execute(request))
    }

    fn send_blocking(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        if Handle::try_current().is_ok() {
            return Err(HttpError::BlockingInAsyncRuntime);
        }
        self.execute_blocking(&request)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClientKey {
    follow_redirects: bool,
    tls: TlsVerification,
    certificate: Option<(String, String)>,
}

impl ClientKey {
    fn of(request: &HttpRequest) -> Self {
        Self {
            follow_redirects: request.follow_redirects,
            tls: request.tls.clone(),
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
    fn cached(&self, key: &ClientKey) -> Option<T> {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn store(&self, key: &ClientKey, client: T) {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), client);
    }

    fn get<Build>(&self, key: &ClientKey, build: Build) -> Result<T, HttpError>
    where
        Build: FnOnce(&ClientKey) -> Result<T, HttpError>,
    {
        if let Some(cached) = self.cached(key) {
            return Ok(cached);
        }
        let client = build(key)?;
        self.store(key, client.clone());
        Ok(client)
    }
}

impl Clients<BlockingClient> {
    fn handle(&self, request: &HttpRequest) -> Result<BlockingClient, HttpError> {
        self.get(&ClientKey::of(request), |client_key| {
            BlockingClientBuilder::from(blocking_builder(client_key)?)
                .build()
                .map_err(HttpError::Client)
        })
    }
}

fn base_builder(key: &ClientKey) -> ClientBuilder {
    let builder = ClientBuilder::new().redirect(redirect_policy(key.follow_redirects));
    if key.tls == TlsVerification::Disabled {
        builder.tls_danger_accept_invalid_certs(true)
    } else {
        builder
    }
}

fn root_certificates(builder: ClientBuilder, pem: &[u8]) -> Result<ClientBuilder, HttpError> {
    // httpx trusts only the verify bundle; reqwest's add_root_certificate is deprecated in 0.13
    Certificate::from_pem_bundle(pem)
        .map(|certificates| builder.tls_certs_only(certificates))
        .map_err(HttpError::Certificate)
}

fn client_identity(builder: ClientBuilder, pem: &[u8]) -> Result<ClientBuilder, HttpError> {
    Identity::from_pem(pem)
        .map(|identity| builder.identity(identity))
        .map_err(HttpError::Certificate)
}

async fn async_builder(key: &ClientKey) -> Result<ClientBuilder, HttpError> {
    let mut builder = base_builder(key);
    if let TlsVerification::CaFile(path) = &key.tls {
        let pem = async_fs::read(path)
            .await
            .map_err(HttpError::CertificateRead)?;
        builder = root_certificates(builder, &pem)?;
    }
    if let Some((certificate_path, key_path)) = &key.certificate {
        let mut pem = async_fs::read(certificate_path)
            .await
            .map_err(HttpError::CertificateRead)?;
        pem.extend(
            async_fs::read(key_path)
                .await
                .map_err(HttpError::CertificateRead)?,
        );
        builder = client_identity(builder, &pem)?;
    }
    Ok(builder)
}

fn blocking_builder(key: &ClientKey) -> Result<ClientBuilder, HttpError> {
    let mut builder = base_builder(key);
    if let TlsVerification::CaFile(path) = &key.tls {
        builder = root_certificates(
            builder,
            &fs::read(path).map_err(HttpError::CertificateRead)?,
        )?;
    }
    let Some((certificate_path, key_path)) = &key.certificate else {
        return Ok(builder);
    };
    let mut pem = fs::read(certificate_path).map_err(HttpError::CertificateRead)?;
    pem.extend(fs::read(key_path).map_err(HttpError::CertificateRead)?);
    client_identity(builder, &pem)
}

fn redirect_policy(follow_redirects: bool) -> Policy {
    if follow_redirects {
        Policy::limited(MAX_REDIRECTS)
    } else {
        Policy::none()
    }
}

const fn method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
        HttpMethod::Delete => Method::DELETE,
    }
}

fn headers(request: &HttpRequest) -> Result<HeaderMap, HttpError> {
    let mut headers = HeaderMap::with_capacity(request.headers.len());
    for (name, value) in &request.headers {
        headers.insert(
            HeaderName::from_str(name).map_err(HttpError::HeaderName)?,
            HeaderValue::from_str(value.expose()).map_err(HttpError::HeaderValue)?,
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

async fn read_streamed_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(HttpError::Transport)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(HttpError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn read_blocking_body(response: BlockingResponse, limit: usize) -> Result<Vec<u8>, HttpError> {
    let take = u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX);
    let mut body = Vec::new();
    response
        .take(take)
        .read_to_end(&mut body)
        .map_err(HttpError::Body)?;
    if body.len() > limit {
        return Err(HttpError::ResponseTooLarge { limit });
    }
    Ok(body)
}
