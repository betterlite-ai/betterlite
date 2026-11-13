use std::io;
use std::time::Duration;

use betterlite_azure::azure_arc_identity::{ArcRequest, ArcResponse, ArcTransport};
use betterlite_azure::azure_client_secret::transport::{
    HttpMethod, HttpReply, HttpRequest, Transport as ClientSecretTransport,
};
use betterlite_azure::azure_hosted_identity::request::{
    HostedRequest, HostedResponse, HostedTransport,
};
use betterlite_azure::azure_managed_identity::imds::{ImdsRequest, ImdsResponse, ImdsTransport};
use betterlite_azure::azure_username_password::transport::{
    HttpRequest as PasswordRequest, Transport as PasswordTransport,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use url::Url;

use betterlite_azure::oidc_exchange::transport::{
    HttpReply as OidcReply, Transport as OidcTransport,
};

const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const METADATA_TIMEOUT: Duration = Duration::from_secs(5);

pub trait AzureIo:
    ClientSecretTransport
    + PasswordTransport
    + OidcTransport
    + ImdsTransport
    + HostedTransport
    + ArcTransport
    + Send
    + Sync
{
}

impl<T> AzureIo for T where
    T: ClientSecretTransport
        + PasswordTransport
        + OidcTransport
        + ImdsTransport
        + HostedTransport
        + ArcTransport
        + Send
        + Sync
{
}

pub struct Exchange<'url> {
    pub method: Method,
    pub url: &'url str,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub deadline: Duration,
}

pub struct AzureHttp {
    pub runtime: Handle,
    pub client: ProxiedClient,
}

impl AzureHttp {
    fn exchange(&self, request: Exchange<'_>) -> io::Result<UpstreamResponse> {
        let url = Url::parse(request.url)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
        let outbound = OutboundRequest {
            method: request.method,
            url,
            query: request.query,
            headers: request.headers,
            body: request.body,
            deadline: request.deadline,
        };
        self.runtime
            .block_on(self.client.send_buffered(outbound))
            .map_err(io::Error::other)
    }

    fn form_exchange(&self, request: Exchange<'_>) -> io::Result<HttpReply> {
        let response = self.exchange(request)?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(HttpReply { status, body })
    }
}

fn password_exchange(request: &PasswordRequest) -> io::Result<Exchange<'_>> {
    let method = request_method(request.method);
    let query = request.query.clone();
    let headers = header_map(request.headers.clone())?;
    let body = form_body(&request.form);
    Ok(Exchange {
        method,
        url: &request.url,
        query,
        headers,
        body,
        deadline: TOKEN_TIMEOUT,
    })
}

fn header_map(headers: Vec<(String, String)>) -> io::Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn form_body(form: &[(String, String)]) -> RequestBody {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            form.iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .finish();
    RequestBody::Bytes {
        content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
        data: encoded.into_bytes(),
    }
}

fn text_reply(status: u16, body: Vec<u8>) -> io::Result<(u16, SecretString)> {
    let text = String::from_utf8(body)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    Ok((status, SecretString::from(text)))
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            pairs.push((name.as_str().to_owned(), text.to_owned()));
        }
    }
    pairs
}

const fn request_method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
    }
}

fn secret_headers(headers: Vec<(&'static str, SecretString)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.expose_secret().to_owned()))
        .collect()
}

impl ClientSecretTransport for AzureHttp {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        let headers = header_map(request.headers.clone())?;
        // authority discovery is a GET and carries no form, so it sends no body at all
        let body = match request.method {
            HttpMethod::Get => RequestBody::Empty,
            HttpMethod::Post => form_body(&request.form),
        };
        self.form_exchange(Exchange {
            method: request_method(request.method),
            url: &request.url,
            query: request.query.clone(),
            headers,
            body,
            deadline: TOKEN_TIMEOUT,
        })
    }
}

impl PasswordTransport for AzureHttp {
    fn send(&self, request: &PasswordRequest) -> io::Result<HttpReply> {
        let exchange = password_exchange(request)?;
        self.form_exchange(exchange)
    }
}

impl OidcTransport for AzureHttp {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<OidcReply> {
        let fields: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let reply = self.form_exchange(Exchange {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: form_body(&fields),
            deadline: TOKEN_TIMEOUT,
        })?;
        Ok(OidcReply {
            status: reply.status,
            body: reply.body,
        })
    }
}

impl ImdsTransport for AzureHttp {
    fn get(&self, request: &ImdsRequest) -> io::Result<ImdsResponse> {
        let response = self.exchange(Exchange {
            method: Method::GET,
            url: &request.url,
            query: request.query.clone(),
            headers: header_map(request.headers.clone())?,
            body: RequestBody::Empty,
            deadline: Duration::from_secs(request.timeout_seconds),
        })?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(ImdsResponse {
            status,
            headers: header_pairs(&response.headers),
            body,
        })
    }
}

impl ArcTransport for AzureHttp {
    fn get(&self, request: &ArcRequest) -> io::Result<ArcResponse> {
        let query = request
            .query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        let headers = header_map(secret_headers(request.headers.clone()))?;
        let response = self.exchange(Exchange {
            method: Method::GET,
            url: &request.url,
            query,
            headers,
            body: RequestBody::Empty,
            deadline: METADATA_TIMEOUT,
        })?;
        let (status, body) = text_reply(response.status, response.body)?;
        let headers = header_pairs(&response.headers);
        Ok(ArcResponse {
            status,
            headers,
            body,
        })
    }
}

impl HostedTransport for AzureHttp {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse> {
        let response = self.exchange(Exchange {
            method: request_method(request.method),
            url: &request.url,
            query: request.query.clone(),
            headers: header_map(secret_headers(request.headers.clone()))?,
            body: form_body(&request.form),
            deadline: METADATA_TIMEOUT,
        })?;
        if response.status != 200 {
            return Err(io::Error::other(format!(
                "the managed identity endpoint returned HTTP status {}",
                response.status
            )));
        }
        let content_type = response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let (_status, body) = text_reply(response.status, response.body)?;
        Ok(HostedResponse { content_type, body })
    }
}
