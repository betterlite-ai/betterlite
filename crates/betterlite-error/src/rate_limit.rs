use std::collections::BTreeMap;
use std::fmt;

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use betterlite_redact::structured::Payload;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitCategory {
    VendorRateLimit,
    VendorBatchRateLimit,
    LitellmRateLimit,
    LitellmBatchRateLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitDimension {
    Requests,
    Tokens,
    ConcurrentRequests,
    Budget,
    MaxIterations,
}

// litellm.constants.PROXY_LLM_PROVIDER_FALLBACK: the provider label a rate-limit model gets when it can't be resolved.
pub const PROXY_LLM_PROVIDER_FALLBACK: &str = "litellm_proxy";

// ProxyRateLimitError.__init__'s six arguments, before its defaults fill the model and provider.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProxyRateLimitErrorInit {
    pub detail: Payload,
    pub headers: Option<BTreeMap<String, Payload>>,
    pub category: Option<RateLimitCategory>,
    pub rate_limit_type: Option<RateLimitDimension>,
    pub model: Option<String>,
    pub llm_provider: Option<String>,
}

// A 429 from litellm's proxy-side limiting: the HTTPException side a route handler forwards and the RateLimitError side callers categorize.
#[derive(Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ProxyRateLimitError {
    pub detail: Payload,
    pub message: String,
    pub headers: Option<HeaderMap>,
    pub category: RateLimitCategory,
    pub rate_limit_type: Option<RateLimitDimension>,
    pub model: String,
    pub llm_provider: String,
}

// A header the limiter named that HTTP can't carry; it names the header, never the value, which can be a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidRateLimitHeader {
    #[error("rate limit header name {name:?} is not a valid HTTP header name")]
    Name { name: String },
    #[error("rate limit header {name:?} has a value HTTP cannot carry")]
    Value { name: String },
}

impl ProxyRateLimitError {
    // Prometheus' exception_class label stays HTTPException, which is what proxy-side 429s were before this class.
    pub const PROMETHEUS_EXCEPTION_CLASS: &str = "HTTPException";

    /// # Errors
    ///
    /// Returns [`InvalidRateLimitHeader`] naming the first header whose name or value HTTP cannot carry.
    pub fn new(init: ProxyRateLimitErrorInit) -> Result<Self, InvalidRateLimitHeader> {
        let ProxyRateLimitErrorInit {
            detail,
            headers,
            category,
            rate_limit_type,
            model,
            llm_provider,
        } = init;
        let message = coerce_message(&detail);
        let headers = headers.map(http_headers).transpose()?;
        Ok(Self {
            detail,
            message,
            headers,
            category: category.unwrap_or(RateLimitCategory::LitellmRateLimit),
            rate_limit_type,
            model: model.unwrap_or_default(),
            llm_provider: llm_provider.unwrap_or_else(|| String::from(PROXY_LLM_PROVIDER_FALLBACK)),
        })
    }
}

// never the derived Debug: a limiter's header value must not escape through format!("{:?}")
impl fmt::Debug for ProxyRateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Option<Vec<&str>> = self
            .headers
            .as_ref()
            .map(|headers| headers.keys().map(HeaderName::as_str).collect());
        f.debug_struct("ProxyRateLimitError")
            .field("detail", &self.detail)
            .field("message", &self.message)
            .field("headers", &header_names)
            .field("category", &self.category)
            .field("rate_limit_type", &self.rate_limit_type)
            .field("model", &self.model)
            .field("llm_provider", &self.llm_provider)
            .finish()
    }
}

// FastAPI's http_exception_handler writes {"detail": exc.detail} with the exception's status and headers.
#[derive(serde::Serialize)]
struct HttpExceptionBody<'detail> {
    detail: &'detail Payload,
}

impl IntoResponse for ProxyRateLimitError {
    fn into_response(self) -> Response {
        let Self {
            detail, headers, ..
        } = self;
        let body = HttpExceptionBody { detail: &detail };
        let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
        if let Some(present) = headers {
            response.headers_mut().extend(present);
        }
        response
    }
}

// _coerce_message: None is "", a string passes through, a mapping yields its "error" or "message" text, else the value's JSON.
#[must_use]
pub fn coerce_message(detail: &Payload) -> String {
    match detail {
        Payload::Null => String::new(),
        Payload::String(text) => text.clone(),
        Payload::Object(fields) => {
            for key in ["error", "message"] {
                if let Some(Payload::String(text)) = fields.get(key) {
                    return text.clone();
                }
                if let Some(Payload::Object(inner)) = fields.get(key)
                    && let Some(Payload::String(text)) = inner.get("message")
                {
                    return text.clone();
                }
            }
            json_text(detail)
        }
        other @ (Payload::Bool(_) | Payload::Number(_) | Payload::Array(_)) => other.to_string(),
    }
}

// json.dumps in LiteLLM, in the compact JSON betterlite writes instead of Python's spacing (docs/compatibility.md).
fn json_text(detail: &Payload) -> String {
    match serde_json::to_string(detail) {
        Ok(text) => text,
        Err(_error) => detail.to_string(),
    }
}

// {k: str(v)} in LiteLLM, checked against what HTTP can carry; names that differ only by case stay two entries.
fn http_headers(headers: BTreeMap<String, Payload>) -> Result<HeaderMap, InvalidRateLimitHeader> {
    let mut checked = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_error| InvalidRateLimitHeader::Name { name: name.clone() })?;
        let header_value = HeaderValue::from_str(&stringified_header(&value))
            .map_err(|_error| InvalidRateLimitHeader::Value { name })?;
        checked.append(header_name, header_value);
    }
    Ok(checked)
}

fn stringified_header(value: &Payload) -> String {
    match value {
        Payload::String(text) => text.clone(),
        other @ (Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Object(_)
        | Payload::Array(_)) => other.to_string(),
    }
}
