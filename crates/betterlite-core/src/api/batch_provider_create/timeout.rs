use serde::Deserialize;

pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// GenericLiteLLMParams.timeout: seconds, or the pieces of an httpx.Timeout
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RequestTimeout {
    Seconds(f64),
    Client(HttpTimeout),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpTimeout {
    pub connect: Option<f64>,
    pub read: Option<f64>,
    pub write: Option<f64>,
    pub pool: Option<f64>,
}

// optional_params.timeout wins over request_timeout and both fall back to 600 seconds; a client
// timeout contributes its read value, and an explicit zero stays zero
pub(super) fn resolve(optional: Option<RequestTimeout>, request_timeout: Option<f64>) -> f64 {
    match optional.or(request_timeout.map(RequestTimeout::Seconds)) {
        Some(RequestTimeout::Seconds(seconds)) => seconds,
        Some(RequestTimeout::Client(client)) => client.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        None => DEFAULT_TIMEOUT_SECONDS,
    }
}
