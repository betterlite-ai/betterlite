use reqwest::header::{
    AUTHORIZATION, CONTENT_LENGTH, COOKIE, HOST, HeaderMap, HeaderName, PROXY_AUTHORIZATION,
    TRANSFER_ENCODING, WWW_AUTHENTICATE,
};
use reqwest::{Method, Request, Response, Url};

use crate::proxy::ProxiedClient;
use crate::proxy::limits::read_bounded;
use crate::proxy::transport_error::TransportError;
use crate::url_policy::origin::Origin;
use crate::user_url::error::UserUrlError;
use crate::user_url::httpx_url::HttpxUrl;
use crate::user_url::redirect::{location, redirect_url};

pub(crate) const MAX_REDIRECTS: usize = 10;
const REDIRECT_BODY_BYTES: usize = 16 * 1024;

const SECRET_HEADERS: [HeaderName; 8] = [
    AUTHORIZATION,
    COOKIE,
    PROXY_AUTHORIZATION,
    WWW_AUTHENTICATE,
    HeaderName::from_static("cookie2"),
    HeaderName::from_static("api-key"),
    HeaderName::from_static("x-api-key"),
    HeaderName::from_static("x-goog-api-key"),
];

fn redirect_method(status: u16, method: &Method) -> Method {
    let becomes_get = match status {
        301 => *method == Method::POST,
        302 | 303 => *method != Method::HEAD,
        _ => false,
    };
    if becomes_get {
        Method::GET
    } else {
        method.clone()
    }
}

// httpx follows five statuses, and only when a Location header says where to
fn redirect_location(response: &Response) -> Option<String> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    location(response.headers())
}

// httpx Client._redirect_url plus betterlite's refusals: no userinfo, no other scheme, no TLS downgrade
fn next_target(current: &HttpxUrl, target_location: &str) -> Result<HttpxUrl, TransportError> {
    let target = redirect_url(current, target_location).map_err(|source| {
        TransportError::RedirectTarget {
            source: Box::new(source),
        }
    })?;
    if !target.userinfo.is_empty() {
        return Err(TransportError::RedirectUserinfo);
    }
    if current.scheme == "https" && target.scheme == "http" {
        return Err(TransportError::InsecureRedirect);
    }
    if target.scheme != "http" && target.scheme != "https" {
        return Err(TransportError::RedirectScheme {
            scheme: target.scheme,
        });
    }
    Ok(target)
}

fn same_origin(current: &HttpxUrl, target: &HttpxUrl) -> bool {
    match (
        Origin::parse(&current.href()),
        Origin::parse(&target.href()),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn strip_credentials(headers: &mut HeaderMap) {
    for name in SECRET_HEADERS {
        headers.remove(name);
    }
    let sensitive: Vec<HeaderName> = headers
        .iter()
        .filter(|(_, value)| value.is_sensitive())
        .map(|(name, _)| name.clone())
        .collect();
    for name in sensitive {
        headers.remove(name);
    }
}

fn next_request(
    request: Request,
    method: &Method,
    current: &HttpxUrl,
    target: &HttpxUrl,
    target_url: Url,
) -> Request {
    let changed = *method != *request.method();
    let mut next = if changed {
        let mut fresh = Request::new(method.clone(), request.url().clone());
        *fresh.headers_mut() = request.headers().clone();
        *fresh.timeout_mut() = request.timeout().copied();
        fresh
    } else {
        request
    };
    *next.url_mut() = target_url;
    if changed && *method == Method::GET {
        next.headers_mut().remove(CONTENT_LENGTH);
        next.headers_mut().remove(TRANSFER_ENCODING);
    }
    if !same_origin(current, target) {
        next.headers_mut().remove(HOST);
        strip_credentials(next.headers_mut());
    }
    next
}

// reqwest's Policy cannot rewrite a request, so each hop is rebuilt here with its headers filtered
pub(crate) async fn send(
    client: &ProxiedClient,
    mut request: Request,
) -> Result<Response, TransportError> {
    let mut current = HttpxUrl::parse(request.url().as_str()).map_err(|source| {
        TransportError::RedirectTarget {
            source: Box::new(UserUrlError::InvalidUrl(source)),
        }
    })?;
    let mut hops = 0;
    loop {
        if let Some(rejection) = client.rejected(request.url()) {
            return Err(TransportError::Rejected(rejection));
        }
        let replay = request.try_clone();
        let mut response = client
            .client
            .execute(request)
            .await
            .map_err(TransportError::sending)?;
        let status = response.status().as_u16();
        let Some(target_location) = redirect_location(&response) else {
            return Ok(response);
        };
        if hops == MAX_REDIRECTS {
            return Err(TransportError::TooManyRedirects {
                limit: MAX_REDIRECTS,
            });
        }
        let target = next_target(&current, &target_location)?;
        let target_url =
            Url::parse(&target.href()).map_err(|source| TransportError::RedirectUrl { source })?;
        let _drained = read_bounded(&mut response, REDIRECT_BODY_BYTES).await;
        let template = replay.ok_or(TransportError::RedirectBody)?;
        let method = redirect_method(status, template.method());
        request = next_request(template, &method, &current, &target, target_url);
        current = target;
        hops = hops.saturating_add(1);
    }
}
