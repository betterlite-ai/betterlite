use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;

pub fn set_scim_content_type(response: &mut Response) {
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/scim+json"),
    );
}
