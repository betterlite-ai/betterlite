use super::cases::{batch_request_case, off_loop_case, request_target_case, service_request_case};
use super::input::{
    Call, CredentialInput, DataInput, JsonPost as JsonPostInput,
    RequestHeaders as RequestHeadersInput,
};
use crate::helpers::{HelperError, OrHelperError as _};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::signing::body::Body;
use betterlite_bedrock::signing::endpoint::{RequestHeaders as EndpointRequest, request_headers};
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::json_post::{JsonPost, sign_aws_json_post};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::{Builder, Runtime};
pub(super) fn run_call(call: &Call) -> Value {
    match call {
        Call::RequestHeaders(case) => step(request_headers_case(case)),
        Call::ServiceRequest(case) => step(service_request_case(case)),
        Call::JsonPost(case) => step(json_post_case(case)),
        Call::RequestTarget(case) => step(request_target_case(case)),
        Call::BatchRequest(case) => step(batch_request_case(case)),
        Call::OffLoop(case) => step(off_loop_case(case)),
    }
}
fn step(result: Result<Value, Box<HelperError>>) -> Value {
    match result {
        Ok(value) => value,
        Err(error) => json!({"ok": false, "status": null, "message": error.message}),
    }
}
fn request_headers_case(case: &RequestHeadersInput) -> Result<Value, Box<HelperError>> {
    let credentials = case.credentials.as_ref().map(credentials_of);
    let bytes;
    let data = match &case.data {
        DataInput::Text(text) => Body::Text(text),
        DataInput::Base64(encoded) => {
            bytes = STANDARD.decode(encoded).or_value_error()?;
            Body::Bytes(&bytes)
        }
    };
    let headers: Vec<_> = case.headers.clone().into_iter().collect();
    let extra: Vec<_> = case.extra_headers.clone().into_iter().collect();
    let request = EndpointRequest {
        credentials: credentials.as_ref(),
        region: &case.region,
        extra_headers: &extra,
        endpoint: &case.endpoint,
        data,
        headers: &headers,
        api_key: case.api_key.as_ref(),
        configured_bearer: case.configured_bearer_token.as_ref(),
        supports_bearer_token: case.supports_bearer_token,
        now: epoch(case.now)?,
    };
    Ok(signed_result(request_headers(request)))
}
fn json_post_case(case: &JsonPostInput) -> Result<Value, Box<HelperError>> {
    let credentials = credentials_of(&case.credentials);
    let headers: Vec<_> = case.headers.clone().into_iter().collect();
    Ok(signed_result(sign_aws_json_post(JsonPost {
        service_name: &case.service_name,
        region: case.region.as_deref(),
        url: &case.url,
        body: &case.body,
        headers: &headers,
        credentials: &credentials,
        now: epoch(case.now)?,
    })))
}
pub(super) fn signed_result(result: Result<AwsSignedRequest, SigningError>) -> Value {
    match result {
        Ok(signed) => signed_json(&signed),
        Err(_error) => json!({"ok": false, "status": null}),
    }
}
pub(super) fn credentials_of(input: &CredentialInput) -> Credentials {
    Credentials {
        access_key_id: input.access_key_id.clone(),
        secret_access_key: input.secret_access_key.clone(),
        session_token: input.session_token.clone(),
    }
}
pub(super) fn credentials_json(credentials: &Credentials) -> Value {
    json!([
        credentials.access_key_id.expose_secret(),
        credentials.secret_access_key.expose_secret(),
        credentials
            .session_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
    ])
}
pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}
pub(super) fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}
fn signed_json(signed: &AwsSignedRequest) -> Value {
    json!({
        "ok": true,
        "method": &signed.method,
        "url": &signed.url,
        "headers": signed.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
        "body": STANDARD.encode(&signed.body),
    })
}
