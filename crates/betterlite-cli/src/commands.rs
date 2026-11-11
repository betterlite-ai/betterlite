pub mod chat;
pub mod configure;
pub mod credentials;
pub mod debug;
pub mod encryption;
pub mod http;
pub mod keys;
pub mod model_groups;
pub mod models;
mod pkce_transport;
mod tables;
pub mod teams;
pub mod users;

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use ::http::Method;
use ::http::header::{
    AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderName,
    InvalidHeaderValue,
};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};
use url::Url;

use crate::cli_context::CliContext;
use crate::pkce_login::HttpResponse;

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const BODY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

// requests reads these from the process environment, and ProxyPolicy::resolve expects the same names in its map.
const PROXY_VARIABLES: [&str; 12] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "AIOHTTP_TRUST_ENV",
    "DISABLE_AIOHTTP_TRUST_ENV",
    "NETRC",
    "HOME",
];

#[derive(Debug)]
pub enum Payload {
    Empty,
    Json(Vec<u8>),
    Form(Vec<(String, SecretString)>),
    Raw(Vec<u8>),
}

pub struct Request {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub payload: Payload,
}

struct Exchange<'request> {
    transport: &'request ProxiedClient,
    request: Request,
    timeout: Duration,
    authorization: Option<&'request SecretString>,
}

struct PreparedRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
    timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot resolve the proxy settings")]
    ProxySettings(#[source] betterlite_env::EnvError),
    #[error("cannot build the HTTP client")]
    Client(#[source] HttpError),
    #[error("cannot start the HTTP runtime")]
    Runtime(#[source] io::Error),
    #[error("{url} is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("{name} is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the value of the {name} header cannot be sent")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    Transport(#[from] HttpError),
}

pub struct Client {
    http: ProxiedClient,
    without_redirects: ProxiedClient,
    runtime: Runtime,
    base_url: String,
    pub(crate) api_key: Option<SecretString>,
}

impl Client {
    pub fn new(
        context: &CliContext,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, ClientError> {
        let resolved =
            ProxyPolicy::resolve(&ProxyTrust::default(), &proxy_environment(environment))
                .map_err(ClientError::ProxySettings)?;
        let policy = Arc::new(resolved);
        let http = ProxiedClient::outbound(&policy).map_err(ClientError::Client)?;
        let without_redirects =
            ProxiedClient::outbound_without_redirects(&policy).map_err(ClientError::Client)?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(ClientError::Runtime)?;
        Ok(Self {
            http,
            without_redirects,
            runtime,
            base_url: context.base_url.trim_end_matches('/').to_owned(),
            api_key: context.api_key.clone(),
        })
    }

    pub fn resolve(&self, uri: &str) -> Result<Url, ClientError> {
        let full = format!("{}/{}", self.base_url, uri.trim_start_matches('/'));
        match Url::parse(&full) {
            Ok(url) => Ok(url),
            Err(source) => Err(ClientError::Url { url: full, source }),
        }
    }

    pub fn send(&self, request: Request) -> Result<HttpResponse, ClientError> {
        self.exchange(Exchange {
            transport: &self.http,
            request,
            timeout: REQUEST_TIMEOUT,
            authorization: self.api_key.as_ref(),
        })
    }

    fn exchange(&self, exchange: Exchange<'_>) -> Result<HttpResponse, ClientError> {
        let Exchange {
            transport,
            request,
            timeout,
            authorization,
        } = exchange;
        let Request {
            method,
            url,
            headers,
            payload,
        } = request;
        let mut header_map = build_headers(headers, authorization)?;
        let body = encode_payload(payload, &mut header_map);
        let prepared = PreparedRequest {
            method,
            url,
            headers: header_map,
            body,
            timeout,
        };
        self.send_request(transport, prepared)
            .map_err(ClientError::Transport)
    }

    fn send_request(
        &self,
        transport: &ProxiedClient,
        prepared: PreparedRequest,
    ) -> Result<HttpResponse, HttpError> {
        let PreparedRequest {
            method,
            url,
            headers,
            body,
            timeout,
        } = prepared;
        let builder = transport
            .client
            .request(method, url)
            .timeout(timeout)
            .headers(headers);
        let with_body = if body.is_empty() {
            builder
        } else {
            builder.body(body)
        };
        let built = with_body.build().map_err(HttpError::Request)?;
        let response = self
            .runtime
            .block_on(transport.execute_buffered(built, BODY_LIMIT_BYTES))?;
        Ok(reply(response))
    }
}

fn build_headers(
    headers: Vec<(String, String)>,
    authorization: Option<&SecretString>,
) -> Result<HeaderMap, ClientError> {
    let mut header_map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = match HeaderName::from_bytes(name.as_bytes()) {
            Ok(found) => found,
            Err(source) => return Err(ClientError::HeaderName { name, source }),
        };
        let header_value = match HeaderValue::from_bytes(value.as_bytes()) {
            Ok(found) => found,
            Err(source) => return Err(ClientError::HeaderValue { name, source }),
        };
        let _previous_header = header_map.insert(header_name, header_value);
    }
    if let Some(key) = authorization {
        let bearer = format!("Bearer {}", key.expose_secret());
        let mut value = HeaderValue::from_bytes(bearer.as_bytes()).map_err(|source| {
            ClientError::HeaderValue {
                name: String::from("authorization"),
                source,
            }
        })?;
        value.set_sensitive(true);
        let _previous_bearer = header_map.insert(AUTHORIZATION, value);
    }
    Ok(header_map)
}

fn encode_payload(payload: Payload, header_map: &mut HeaderMap) -> Vec<u8> {
    match payload {
        Payload::Empty => Vec::new(),
        Payload::Raw(body) => body,
        Payload::Json(body) => {
            let _previous_content_type =
                header_map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            body
        }
        Payload::Form(fields) => {
            let _previous_form_type = header_map.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            );
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (name, value) in fields {
                serializer.append_pair(&name, value.expose_secret());
            }
            serializer.finish().into_bytes()
        }
    }
}

fn reply(response: UpstreamResponse) -> HttpResponse {
    let headers = response
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect();
    HttpResponse {
        status: response.status,
        headers,
        body: response.body,
    }
}

fn proxy_environment(environment: &dyn EnvironmentLookup) -> BTreeMap<String, String> {
    PROXY_VARIABLES
        .into_iter()
        .filter_map(|name| {
            environment
                .lookup(name)
                .map(|value| (name.to_owned(), value))
        })
        .collect()
}

mod tests;
