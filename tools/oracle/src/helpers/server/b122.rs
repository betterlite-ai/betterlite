use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Read as _;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use axum::response::Response;
use betterlite_server::middleware::gzip_buffered::GzipBufferedLayer;
use betterlite_server::middleware::security_headers::SecurityHeadersLayer;
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tower::ServiceExt as _;

use crate::helpers::HelperError;

const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    accept_encoding: Option<String>,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body_size: Option<usize>,
    #[serde(default)]
    body_byte: Option<u8>,
    #[serde(default)]
    enable_hsts: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Gzip,
    SecurityHeaders,
}

fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let parsed_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| failure("InvalidInput", error))?;
        let parsed_value =
            HeaderValue::from_str(value).map_err(|error| failure("InvalidInput", error))?;
        map.insert(parsed_name, parsed_value);
    }
    Ok(map)
}

fn header_text(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn send(app: Router, request: Request<Body>) -> Result<Response, Box<HelperError>> {
    app.oneshot(request)
        .await
        .map_err(|error| failure("ServiceError", error))
}

fn get_request() -> Result<Request<Body>, Box<HelperError>> {
    Request::builder()
        .method(Method::GET)
        .uri("/")
        .body(Body::empty())
        .map_err(|error| failure("InvalidRequest", error))
}

async fn run_gzip(input: &Input) -> Result<serde_json::Value, Box<HelperError>> {
    let size = input
        .body_size
        .ok_or_else(|| failure("InvalidInput", "gzip needs body_size"))?;
    let byte = input.body_byte.unwrap_or(97);
    let payload = vec![byte; size];
    let status = StatusCode::from_u16(input.status.unwrap_or(200))
        .map_err(|error| failure("InvalidInput", error))?;
    let mut headers = header_map(&input.headers)?;
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).map_err(|error| failure("InvalidInput", error))?,
    );
    let response_headers = headers.clone();
    let app = Router::new()
        .fallback(move || {
            let generated = response_headers.clone();
            let body = payload.clone();
            async move {
                let mut response = Response::new(Body::from(body));
                *response.status_mut() = status;
                *response.headers_mut() = generated;
                response
            }
        })
        .layer(GzipBufferedLayer);
    let mut builder = Request::builder().method(Method::GET).uri("/");
    if let Some(accept) = &input.accept_encoding {
        builder = builder.header(header::ACCEPT_ENCODING, accept);
    }
    let request = builder
        .body(Body::empty())
        .map_err(|error| failure("InvalidRequest", error))?;
    let response = send(app, request).await?;
    let status = response.status();
    let encoding = header_text(response.headers(), header::CONTENT_ENCODING);
    let vary = header_text(response.headers(), header::VARY);
    let length = header_text(response.headers(), header::CONTENT_LENGTH);
    let body = to_bytes(response.into_body(), MAX_BODY)
        .await
        .map_err(|error| failure("ServiceError", error))?;
    let decoded = if encoding.as_deref() == Some("gzip") {
        let mut decoded = Vec::new();
        GzDecoder::new(body.as_ref())
            .read_to_end(&mut decoded)
            .map_err(|error| failure("DecodeError", error))?;
        decoded
    } else {
        body.to_vec()
    };
    Ok(serde_json::json!({
        "status": status.as_u16(),
        "compressed": encoding.as_deref() == Some("gzip"),
        "content_encoding": encoding,
        "vary": vary,
        "content_length_matches": length == Some(body.len().to_string()),
        "decoded_sha256": hex::encode(Sha256::digest(&decoded)),
        "decoded_length": decoded.len(),
    }))
}

async fn run_security_headers(input: &Input) -> Result<serde_json::Value, Box<HelperError>> {
    let status = StatusCode::from_u16(input.status.unwrap_or(200))
        .map_err(|error| failure("InvalidInput", error))?;
    let enable_hsts = input.enable_hsts.unwrap_or(false);
    let response_headers = header_map(&input.headers)?;
    let app = Router::new()
        .fallback(move || {
            let configured = response_headers.clone();
            async move {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = status;
                *response.headers_mut() = configured;
                response
            }
        })
        .layer(SecurityHeadersLayer { enable_hsts });
    let response = send(app, get_request()?).await?;
    let status = response.status().as_u16();
    Ok(serde_json::json!({
        "status": status,
        "x-frame-options": header_text(response.headers(), header::X_FRAME_OPTIONS),
        "content-security-policy": header_text(response.headers(), header::CONTENT_SECURITY_POLICY),
        "x-content-type-options": header_text(response.headers(), header::X_CONTENT_TYPE_OPTIONS),
        "strict-transport-security": header_text(response.headers(), header::STRICT_TRANSPORT_SECURITY),
    }))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        match input.function {
            Function::Gzip => runtime.block_on(run_gzip(input)),
            Function::SecurityHeaders => runtime.block_on(run_security_headers(input)),
        }
    }
}
