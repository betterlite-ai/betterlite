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

pub struct AzureHttp {
    pub runtime: Handle,
    pub client: ProxiedClient,
}

impl AzureHttp {
    fn execute(
        &self,
        method: Method,
        url: &str,
        query: Vec<(String, String)>,
        headers: HeaderMap,
        body: RequestBody,
        deadline: Duration,
    ) -> io::Result<betterlite_http::proxy::UpstreamResponse> {
        let url = Url::parse(url)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
        let request = OutboundRequest {
            method,
            url,
            query,
            headers,
            body,
            deadline,
        };
        self.runtime
            .block_on(self.client.send_buffered(request))
            .map_err(io::Error::other)
    }
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

fn request_method(method: HttpMethod) -> Method {
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
        let response = self.execute(
            request_method(request.method),
            &request.url,
            request.query.clone(),
            headers,
            form_body(&request.form),
            TOKEN_TIMEOUT,
        )?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(HttpReply { status, body })
    }
}

impl PasswordTransport for AzureHttp {
    fn send(&self, request: &PasswordRequest) -> io::Result<HttpReply> {
        let headers = header_map(request.headers.clone())?;
        let response = self.execute(
            request_method(request.method),
            &request.url,
            request.query.clone(),
            headers,
            form_body(&request.form),
            TOKEN_TIMEOUT,
        )?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(HttpReply { status, body })
    }
}

impl OidcTransport for AzureHttp {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<OidcReply> {
        let form: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let response = self.execute(
            Method::POST,
            url,
            Vec::new(),
            HeaderMap::new(),
            form_body(&form),
            TOKEN_TIMEOUT,
        )?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(OidcReply { status, body })
    }
}

impl ImdsTransport for AzureHttp {
    fn get(&self, request: &ImdsRequest) -> io::Result<ImdsResponse> {
        let headers = header_map(request.headers.clone())?;
        let response = self.execute(
            Method::GET,
            &request.url,
            request.query.clone(),
            headers,
            RequestBody::Empty,
            Duration::from_secs(request.timeout_seconds),
        )?;
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
        let response = self.execute(
            Method::GET,
            &request.url,
            query,
            header_map(secret_headers(request.headers.clone()))?,
            RequestBody::Empty,
            METADATA_TIMEOUT,
        )?;
        let (status, body) = text_reply(response.status, response.body)?;
        Ok(ArcResponse {
            status,
            headers: header_pairs(&response.headers),
            body,
        })
    }
}

impl HostedTransport for AzureHttp {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse> {
        let response = self.execute(
            request_method(request.method),
            &request.url,
            request.query.clone(),
            header_map(secret_headers(request.headers.clone()))?,
            form_body(&request.form),
            METADATA_TIMEOUT,
        )?;
        let (status, body) = text_reply(response.status, response.body)?;
        if status != 200 {
            return Err(io::Error::other(format!(
                "the managed identity endpoint returned HTTP status {status}"
            )));
        }
        let content_type = response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok(HostedResponse { content_type, body })
    }
}
