use std::convert::identity;
use std::error::Error;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use http::header::LOCATION;
use http::{HeaderMap, Method};
use url::Url;

use crate::client::cleanup::http_client::{
    ClientBody, ClientError, HttpResponse, McpHttpClient, ResponseHead, Transport as _,
};
use crate::client::cleanup::recording::ResponseBody;
use crate::client::http_transport::error::McpHttpError;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::{HttpBody, McpHttpTransport};

use super::error::SdkError;
const MAX_REDIRECTS: usize = 20;
fn location_target(sent: &Url, status: u16, headers: &HeaderMap) -> Option<Url> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let raw = headers.get(LOCATION)?.to_str().ok()?;
    Url::parse(raw).map_or_else(|_| sent.join(raw).ok(), Some)
}
fn redirected_method(method: &Method, status: u16) -> Method {
    if status == 303 && *method != Method::HEAD {
        return Method::GET;
    }
    if status == 302 && *method != Method::HEAD && method.as_str() != "QUERY" {
        return Method::GET;
    }
    if status == 301 && *method == Method::POST {
        return Method::GET;
    }
    method.clone()
}
fn brings_userinfo(sent: &Url, target: &Url) -> bool {
    if target.username() == "" && target.password().is_none() {
        return false;
    }
    target.username() != sent.username() || target.password() != sent.password()
}
fn within_origin(sent: &Url, target: &Url) -> bool {
    let same = sent.scheme() == target.scheme()
        && sent.host_str() == target.host_str()
        && sent.port() == target.port();
    if same {
        return true;
    }
    // httpx2 permits the plain http to https upgrade only when both URLs leave the port implicit
    sent.host_str() == target.host_str()
        && sent.scheme() == "http"
        && sent.port().is_none()
        && target.scheme() == "https"
        && target.port().is_none()
}
pub(super) fn next_request_within_origin(
    method: &Method,
    url: &Url,
    status: u16,
    headers: &HeaderMap,
) -> Option<(Method, Url)> {
    let target = location_target(url, status, headers)?;
    let next_method = redirected_method(method, status);
    if next_method != *method || brings_userinfo(url, &target) || !within_origin(url, &target) {
        return None;
    }
    Some((next_method, target))
}
fn redirect_location(url: &Url, status: u16, headers: &HeaderMap) -> Option<String> {
    let mut shown = location_target(url, status, headers)?;
    shown.set_query(None);
    shown.set_fragment(None);
    shown.set_username("").ok()?;
    shown.set_password(None).ok()?;
    Some(shown.to_string())
}
pub(super) fn redirect_note(url: &Url, status: u16, headers: &HeaderMap) -> Option<String> {
    let location = redirect_location(url, status, headers)?;
    Some(format!(" (redirected to {location}; not followed)"))
}
pub(super) trait OriginHop {
    type Body: ResponseBody;
    type Error: Error + Send + 'static;
    fn hop(
        &self,
        request: McpHttpRequest,
    ) -> BoxFuture<'_, Result<HttpResponse<Self::Body>, Self::Error>>;
}
impl OriginHop for McpHttpTransport {
    type Body = HttpBody;
    type Error = McpHttpError;
    fn hop(
        &self,
        request: McpHttpRequest,
    ) -> BoxFuture<'_, Result<HttpResponse<HttpBody>, McpHttpError>> {
        Box::pin(self.send(request, true))
    }
}
impl OriginHop for McpHttpClient<McpHttpTransport> {
    type Body = ClientBody<HttpBody>;
    type Error = ClientError<McpHttpError>;
    fn hop(
        &self,
        request: McpHttpRequest,
    ) -> BoxFuture<'_, Result<HttpResponse<ClientBody<HttpBody>>, ClientError<McpHttpError>>> {
        Box::pin(self.send(request, false))
    }
}
async fn follow_redirects<Hop>(
    hop: &Hop,
    drain: fn(<Hop::Body as ResponseBody>::Error) -> Hop::Error,
    request: McpHttpRequest,
) -> Result<HttpResponse<Hop::Body>, Hop::Error>
where
    Hop: OriginHop + Sync,
{
    let mut method = request.method.clone();
    let mut url = request.url.clone();
    let mut followed = 0_usize;
    loop {
        let attempt = McpHttpRequest {
            method: method.clone(),
            url: url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            read_timeout: request.read_timeout,
        };
        let mut response = hop.hop(attempt).await?;
        let next = next_request_within_origin(&method, &url, response.status, &response.headers);
        let Some((next_method, next_url)) = next else {
            return Ok(response);
        };
        if followed == MAX_REDIRECTS {
            return Ok(response);
        }
        drop(response.body.read_all().await.map_err(drain)?);
        response.body.close().await.map_err(drain)?;
        method = next_method;
        url = next_url;
        followed = followed.saturating_add(1);
    }
}
pub(super) async fn send_within_origin(
    transport: &McpHttpTransport,
    request: McpHttpRequest,
) -> Result<HttpResponse<HttpBody>, SdkError> {
    follow_redirects(transport, identity, request)
        .await
        .map_err(SdkError::from)
}
pub(super) async fn delete_within_origin(
    client: &McpHttpClient<McpHttpTransport>,
    request: McpHttpRequest,
) -> Result<u16, ClientError<McpHttpError>> {
    follow_redirects(client, ClientError::Transport, request)
        .await
        .map(|response| response.status)
}
pub(super) fn status_error(response: HttpResponse<HttpBody>) -> SdkError {
    SdkError::Http(Arc::new(McpHttpError::Status(Box::new(ResponseHead {
        status: response.status,
        headers: response.headers,
        capture: response.capture,
    }))))
}
pub(super) fn legacy_failure(url: &Url, response: HttpResponse<HttpBody>) -> SdkError {
    if let Some(note) = redirect_note(url, response.status, &response.headers) {
        return SdkError::Redirect {
            message: note.trim_start().to_owned(),
        };
    }
    status_error(response)
}
