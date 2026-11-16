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
    pub headers: Option<BTreeMap<String, String>>,
    pub category: RateLimitCategory,
    pub rate_limit_type: Option<RateLimitDimension>,
    pub model: String,
    pub llm_provider: String,
}

impl ProxyRateLimitError {
    // Prometheus' exception_class label stays HTTPException, which is what proxy-side 429s were before this class.
    pub const PROMETHEUS_EXCEPTION_CLASS: &str = "HTTPException";

    #[must_use]
    pub fn new(init: ProxyRateLimitErrorInit) -> Self {
        let ProxyRateLimitErrorInit {
            detail,
            headers,
            category,
            rate_limit_type,
            model,
            llm_provider,
        } = init;
        let message = coerce_message(&detail);
        Self {
            detail,
            message,
            headers: headers.map(stringified_headers),
            category: category.unwrap_or(RateLimitCategory::LitellmRateLimit),
            rate_limit_type,
            model: model.unwrap_or_default(),
            llm_provider: llm_provider.unwrap_or_else(|| String::from(PROXY_LLM_PROVIDER_FALLBACK)),
        }
    }
}

// never the derived Debug: a limiter's header value must not escape through format!("{:?}")
impl fmt::Debug for ProxyRateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Option<Vec<&String>> = self
            .headers
            .as_ref()
            .map(|headers| headers.keys().collect());
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
            insert_headers(response.headers_mut(), &present);
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

// {k: str(v)} in LiteLLM; a string stays as it is and every other value keeps its JSON text.
fn stringified_headers(headers: BTreeMap<String, Payload>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| (name, stringified_header(&value)))
        .collect()
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

// A name or value HTTP can't carry is dropped here; the 429 still goes out with its status and body.
fn insert_headers(target: &mut HeaderMap, headers: &BTreeMap<String, String>) {
    for (header_name, header_value) in headers {
        let Ok(name) = HeaderName::from_bytes(header_name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(header_value) else {
            continue;
        };
        target.insert(name, value);
    }
}
