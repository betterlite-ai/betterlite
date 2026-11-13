use axum::body::Body;
use axum::extract::OriginalUri;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Redirect, Response};

use super::redirect_location;

pub(super) async fn slash_redirect(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> Response {
    let mut target = uri
        .path()
        .strip_suffix('/')
        .map_or_else(|| format!("{}/", uri.path()), str::to_owned);
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok());
    Redirect::temporary(&redirect_location(host, &target)).into_response()
}

pub(super) async fn method_not_allowed() -> Response {
    let mut response = detail_response(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    response
}

// the root redirect is a FastAPI get route, which lists only GET in its Allow header
pub(super) async fn root_method_not_allowed() -> Response {
    let mut response = detail_response(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET"));
    response
}

pub(super) fn detail_response(status: StatusCode, detail: &str) -> Response {
    let body = format!("{{\"detail\":\"{detail}\"}}");
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
