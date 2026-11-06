use std::time::Duration;

use indexmap::IndexMap;
use serde::Serialize;

use super::json_tools::json_stringify;
use super::json_value::JsonValue;
use super::urls::is_valid_url;

pub mod transport;
use transport::{
    HttpMethod, HttpReply, OutboundHttp, OutboundRequest, RequestBody, TransportFailure,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_TIMEOUT_SECONDS: f64 = 30.0;
const MIN_TIMEOUT_SECONDS: f64 = 0.1;
const MAX_TIMEOUT_SECONDS: f64 = 60.0;
const ALLOWED_METHODS: &str = "GET, POST, PUT, DELETE, PATCH";

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: IndexMap<String, String>,
    pub body: JsonValue,
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HttpResponseResult {
    pub status_code: u16,
    pub body: JsonValue,
    pub headers: IndexMap<String, String>,
    pub success: bool,
    pub error: Option<String>,
}

pub async fn http_request<Transport>(
    transport: &Transport,
    request: HttpRequest,
) -> HttpResponseResult
where
    Transport: OutboundHttp,
{
    if !is_valid_url(&request.url) {
        return failure(format!("Invalid URL: {}", request.url));
    }
    let method_name = request.method.to_uppercase();
    let Some(method) = HttpMethod::from_name(&method_name) else {
        return failure(format!(
            "Invalid HTTP method: {method_name}. Allowed: {ALLOWED_METHODS}"
        ));
    };
    let seconds = request
        .timeout
        .map_or(DEFAULT_TIMEOUT_SECONDS, |requested| {
            MIN_TIMEOUT_SECONDS.max(requested).min(MAX_TIMEOUT_SECONDS)
        });
    let timeout = Duration::try_from_secs_f64(seconds).unwrap_or(DEFAULT_TIMEOUT);
    match execute(transport, method, request, timeout).await {
        Ok(reply) => success(reply),
        Err(TransportFailure::Refused { message }) => {
            tracing::warn!(%message, "custom code http_request blocked");
            failure(format!("Blocked URL: {message}"))
        }
        Err(TransportFailure::TimedOut) => {
            tracing::warn!("custom code http_request timeout");
            let shown = seconds.to_string();
            let python_float = if shown.contains('.') {
                shown
            } else {
                format!("{shown}.0")
            };
            failure(format!("Request timeout after {python_float}s"))
        }
        Err(TransportFailure::Failed { message }) => {
            tracing::warn!(%message, "custom code http_request error");
            failure(format!("Request failed: {message}"))
        }
    }
}

async fn execute<Transport>(
    transport: &Transport,
    method: HttpMethod,
    request: HttpRequest,
    timeout: Duration,
) -> Result<HttpReply, TransportFailure>
where
    Transport: OutboundHttp,
{
    let validating = transport.user_url_validation();
    let redirects = !validating;
    let mut headers: Vec<(String, String)> = request.headers.into_iter().collect();
    if validating {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("host"));
    }
    let mut outbound = OutboundRequest {
        method,
        url: request.url,
        headers,
        body: RequestBody::Empty,
        timeout,
        connect_timeout: CONNECT_TIMEOUT,
        follow_redirects: redirects,
    };
    if method == HttpMethod::Get {
        return transport.safe_get(outbound).await;
    }
    if validating {
        let destination = transport.validate_url(&outbound.url).await?;
        outbound
            .headers
            .push(("Host".to_owned(), destination.host_header));
        outbound.url = destination.url;
    }
    outbound.body = match request.body {
        JsonValue::Null => RequestBody::Empty,
        JsonValue::Object(fields) => RequestBody::Json(fields),
        JsonValue::String(text) => RequestBody::Text(text),
        other @ (JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_)) => {
            RequestBody::Text(json_stringify(&other))
        }
    };
    transport.send(outbound).await
}

fn success(reply: HttpReply) -> HttpResponseResult {
    let body = serde_json::from_str(&reply.body).unwrap_or(JsonValue::String(reply.body));
    let mut headers: IndexMap<String, String> = IndexMap::new();
    for (name, value) in reply.headers {
        headers
            .entry(name.to_lowercase())
            .and_modify(|joined| joined.extend([", ", value.as_str()]))
            .or_insert(value);
    }
    HttpResponseResult {
        status_code: reply.status,
        body,
        headers,
        success: (200..300).contains(&reply.status),
        error: None,
    }
}

fn failure(error: String) -> HttpResponseResult {
    HttpResponseResult {
        status_code: 0,
        body: JsonValue::Null,
        headers: IndexMap::new(),
        success: false,
        error: Some(error),
    }
}
