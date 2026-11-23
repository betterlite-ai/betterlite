use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::StatusError;
use http::Method;
use http::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::error::AzureSpeechError;
use super::wire::compose_url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const PROBE_PATH: &str = "chat/completions";
const CLOUDFLARE: &str = "gateway.ai.cloudflare.com";
const RATE_LIMIT_HEADERS: [&str; 3] = [
    "x-ratelimit-remaining-requests",
    "x-ratelimit-remaining-tokens",
    "x-ms-region",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeMode {
    Chat,
    ImageGeneration,
}

// AzureChatCompletion.get_headers' arguments, with the caller's session left to whoever sends the request.
pub struct ProbeCall<'data> {
    pub model: Option<&'data str>,
    pub api_key: &'data str,
    pub api_base: Option<&'data str>,
    pub api_version: Option<&'data str>,
    pub timeout: Option<f64>,
    pub mode: ProbeMode,
    pub messages: Option<&'data Json>,
}

// The live rate-limit probe: always a chat completions create, whose only reply fields are the three headers.
pub fn probe_request(call: &ProbeCall<'_>) -> Result<OutboundRequest, AzureSpeechError> {
    let (base, deployment) = probe_target(call)?;
    let url = compose_url(&base, deployment, PROBE_PATH, call.api_version, &[])?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: probe_headers(call.api_key)?,
        body: RequestBody::Json(probe_body(call.model, call.messages)?),
        deadline: probe_timeout(call.timeout)?,
    })
}

pub fn finish_probe(
    answer: Option<UpstreamResponse>,
) -> Result<IndexMap<String, String>, AzureSpeechError> {
    let Some(reply) = answer else {
        return Err(AzureSpeechError::InvalidCompletion);
    };
    if !(200..300).contains(&reply.status) {
        return Err(AzureSpeechError::Status(Box::new(
            StatusError::from_response(reply.status, &reply.body),
        )));
    }
    Ok(rate_limit_headers(&reply.headers))
}

// The gateway needs the model in the path and model=None in the body; every other base prefixes the deployment.
fn probe_target<'data>(
    call: &ProbeCall<'data>,
) -> Result<(String, Option<&'data str>), AzureSpeechError> {
    let Some(api_base) = call.api_base else {
        return Err(AzureSpeechError::MissingBase);
    };
    if api_base.contains(CLOUDFLARE) {
        let mut gateway = api_base.to_owned();
        if !gateway.ends_with('/') {
            gateway.push('/');
        }
        gateway.push_str(call.model.unwrap_or("None"));
        return Ok((gateway, None));
    }
    if call.model.is_none() && call.mode != ProbeMode::ImageGeneration {
        return Err(AzureSpeechError::ModelNotSet);
    }
    Ok((
        format!("{}/openai", api_base.trim_end_matches('/')),
        call.model,
    ))
}

fn probe_headers(api_key: &str) -> Result<HeaderMap, AzureSpeechError> {
    if api_key.is_empty() {
        return Err(AzureSpeechError::UnableToHandleAuth);
    }
    let mut value =
        HeaderValue::try_from(api_key).map_err(|source| AzureSpeechError::HeaderValue {
            name: String::from("api-key"),
            source,
        })?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(HeaderName::from_static("api-key"), value);
    Ok(headers)
}

fn probe_body(model: Option<&str>, messages: Option<&Json>) -> Result<Vec<u8>, AzureSpeechError> {
    let default_messages = Json::List(vec![Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::Text(String::from("Hey"))),
    ]))]);
    let body = IndexMap::from([
        (
            String::from("model"),
            model.map_or(Json::Null, |name| Json::Text(name.to_owned())),
        ),
        (
            String::from("messages"),
            messages.cloned().unwrap_or(default_messages),
        ),
    ]);
    serde_json::to_vec(&body).map_err(AzureSpeechError::Encode)
}

fn probe_timeout(timeout: Option<f64>) -> Result<Duration, AzureSpeechError> {
    timeout.map_or_else(
        || Ok(DEFAULT_TIMEOUT),
        |seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|_invalid| AzureSpeechError::Option {
                option: String::from("timeout"),
                problem: "is not a number of seconds",
            })
        },
    )
}

fn rate_limit_headers(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut found = IndexMap::new();
    for name in RATE_LIMIT_HEADERS {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            found.insert(name.to_owned(), value.to_owned());
        }
    }
    found
}
