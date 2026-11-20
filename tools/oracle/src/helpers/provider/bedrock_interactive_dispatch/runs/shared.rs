pub(crate) use std::collections::BTreeMap;
pub(crate) use std::future::{Future, ready};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

pub(crate) use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_http::proxy::transport_error::TransportError;
pub(crate) use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
pub(crate) use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::bedrock_interactive_dispatch::http::{
    LeafReply, LeafRequest, LeafTransport,
};
pub(crate) use indexmap::IndexMap;
pub(crate) use secrecy::SecretString;
pub(crate) use serde_json::{Map, Value, json};

pub(crate) const RESPONSES_HOST: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";
pub(crate) const GATEWAY: &str = "https://gateway.example.com/mcp";
pub(crate) const VECTOR_ID: &str = "KB123";
pub(crate) const PASSTHROUGH_MODEL: &str = "anthropic.claude-3";
pub(crate) const REALTIME_MODEL: &str = "amazon.nova-sonic-v1:0";
pub(crate) const DEADLINE: Duration = Duration::from_mins(10);
const ACCESS: &str = "AKIDBETTERLITEFIXTURE";
const SECRET: &str = "fixture/secret/key/+EXAMPLEKEY0000";
pub(crate) const CATALOG: &str =
    r#"{"openai.gpt-oss-120b": {"supported_endpoints": ["/v1/responses"]}}"#;

pub(crate) fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

pub(crate) fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from(ACCESS.to_owned()),
        secret_access_key: SecretString::from(SECRET.to_owned()),
        session_token: None,
    }
}

pub(crate) fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    Ok(runtime.block_on(future))
}

pub(crate) fn reply(status: u16, body: &str) -> LeafReply {
    LeafReply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
        chunks: Vec::new(),
    }
}

pub(crate) struct Scripted(pub(crate) LeafReply);

impl LeafTransport for Scripted {
    fn send(
        &self,
        _request: LeafRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LeafReply, TransportError>> + Send + '_>> {
        Box::pin(ready(Ok(self.0.clone())))
    }
}

const ROW_KEYS: [&str; 8] = [
    "request",
    "endpoint_url",
    "streaming",
    "status",
    "reply",
    "response",
    "logged",
    "error",
];

pub(crate) fn empty_row() -> Map<String, Value> {
    ROW_KEYS
        .into_iter()
        .map(|key| (key.to_owned(), Value::Null))
        .collect()
}

pub(crate) fn error_row(kind: &str, status: Option<u16>) -> Value {
    let mut row = empty_row();
    row.insert(
        "error".to_owned(),
        json!({ "kind": kind, "status": status }),
    );
    Value::Object(row)
}

pub(crate) fn request_row(request: &LeafRequest) -> Value {
    json!({
        "method": request.method,
        "url": request.url,
        "auth": auth_view(&request.headers),
        "body": body_json(&request.body),
    })
}

fn auth_view(headers: &[(String, Option<String>)]) -> Value {
    let value = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, raw)| raw.as_deref());
    let Some(text) = value else {
        return Value::Null;
    };
    if let Some(token) = text.strip_prefix("Bearer ") {
        return json!({
            "kind": "bearer",
            "access_key": null,
            "region": null,
            "service": null,
            "token": token,
        });
    }
    let credential = text.split("Credential=").nth(1).unwrap_or_default();
    let scope = credential.split(',').next().unwrap_or_default();
    let parts: Vec<&str> = scope.split('/').collect();
    json!({
        "kind": "sigv4",
        "access_key": parts.first().copied(),
        "region": parts.get(2).copied(),
        "service": parts.get(3).copied(),
        "token": null,
    })
}

pub(crate) fn body_json(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned()))
}

pub(crate) fn success_row(request: &LeafRequest) -> Map<String, Value> {
    let mut row = empty_row();
    row.insert("request".to_owned(), request_row(request));
    row
}

pub(crate) fn outcome_row<T, E>(
    outcome: Result<T, E>,
    failure: fn(&E) -> Value,
    success: fn(&T) -> Value,
) -> Value {
    match outcome {
        Ok(value) => success(&value),
        Err(error) => failure(&error),
    }
}
