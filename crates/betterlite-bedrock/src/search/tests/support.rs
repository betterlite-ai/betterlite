use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::str::from_utf8;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::search::error::SearchError;
use crate::search::request::Params;
use crate::search::signing::{SignRequest, sign_request};
use crate::signing::request::AwsSignedRequest;

pub(super) const NOW_SECONDS: u64 = 1_767_225_600;
pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const OTHER_ACCESS_KEY: &str = "AKIDOTHER";
pub(super) const AWS_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const GATEWAY_URL: &str =
    "https://gateway-abc123.gateway.bedrock-agentcore.us-east-1.amazonaws.com/mcp";
pub(super) const MCP_BODY_TEXT: &str = r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "web-search-tool___WebSearch", "arguments": {"query": "hello", "maxResults": 10}}}"#;
const MCP_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web-search-tool___WebSearch","arguments":{"query":"hello","maxResults":10}}}"#;

pub(super) type Resolution = Ready<Result<Credentials, CredentialError>>;
pub(super) type Resolver = fn(&AwsSettings, &str) -> Resolution;

pub(super) struct Case<'text> {
    pub(super) headers: Headers,
    pub(super) optional_params: Params,
    pub(super) request_data: Json,
    pub(super) api_base: &'text str,
    pub(super) api_key: Option<SecretString>,
    pub(super) session_region: Option<&'text str>,
}

pub(super) fn baseline(api_base: &str) -> Case<'_> {
    Case {
        headers: headers(&[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2025-03-26"),
        ]),
        optional_params: Params::new(),
        request_data: mcp_body(),
        api_base,
        api_key: None,
        session_region: None,
    }
}

pub(super) fn mcp_body() -> Json {
    body(MCP_BODY)
}

pub(super) fn body(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).unwrap()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

pub(super) fn fixture_credentials() -> Credentials {
    credentials(ACCESS_KEY)
}

pub(super) fn credentials(access_key: &str) -> Credentials {
    Credentials {
        access_key_id: secret(access_key),
        secret_access_key: secret(AWS_SECRET),
        session_token: None,
    }
}

pub(super) fn resolver() -> Resolver {
    |_settings: &AwsSettings, _region: &str| ready(Ok(fixture_credentials()))
}

pub(super) fn other_resolver() -> Resolver {
    |_settings: &AwsSettings, _region: &str| ready(Ok(credentials(OTHER_ACCESS_KEY)))
}

pub(super) fn failing() -> Resolver {
    |_settings: &AwsSettings, _region: &str| ready(Err(CredentialError::MissingCredentials))
}

pub(super) fn sign(
    case: &Case<'_>,
    environment: &BTreeMap<String, String>,
    resolve: Resolver,
) -> Result<AwsSignedRequest, SearchError> {
    block_on(sign_request(
        &SignRequest {
            headers: &case.headers,
            optional_params: &case.optional_params,
            request_data: &case.request_data,
            api_base: case.api_base,
            api_key: case.api_key.as_ref(),
            session_region: case.session_region,
            now: now(),
        },
        environment,
        resolve,
    ))
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) fn header<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(found, _)| found == name)
        .map(|(_, value)| value.as_str())
}

pub(super) fn body_text(signed: &AwsSignedRequest) -> &str {
    from_utf8(&signed.body).unwrap()
}

pub(super) fn authorization(signed: &AwsSignedRequest) -> &str {
    header(&signed.headers, "Authorization").unwrap()
}

pub(super) fn plaintext_refusal(api_base: &str) -> String {
    format!(
        "Refusing to send AgentCore credentials over plaintext HTTP to '{api_base}': a bearer \
         token or SigV4 signature would be readable in transit. Use an https gateway URL (plain \
         http is allowed only for localhost)."
    )
}

pub(super) fn text_field<'json>(body: &'json Json, keys: &[&str]) -> &'json str {
    let Json::Text(text) = nested(body, keys) else {
        panic!("the field is not text");
    };
    text
}

pub(super) fn argument_dump(body: &Json, name: &str) -> String {
    dumps(nested(body, &["params", "arguments", name]))
}

fn nested<'json>(body: &'json Json, keys: &[&str]) -> &'json Json {
    let mut current = body;
    for key in keys {
        let Json::Object(object) = current else {
            panic!("the value before {key} is not an object");
        };
        current = object.get(*key).unwrap();
    }
    current
}
