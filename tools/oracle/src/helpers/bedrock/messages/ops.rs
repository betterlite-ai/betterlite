use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::invoke_primitives::url::{InvokeEnvironment, InvokeModelUrl};
use betterlite_bedrock::messages::config::MessagesConfig;
use betterlite_bedrock::messages::stream::{MessagesStreamRequest, messages_sse_stream};
use betterlite_bedrock::messages::transport::{
    MessagesEndpoint, MessagesSignRequest, get_error_class, messages_complete_url,
    sign_messages_request, validate_messages_environment,
};
use betterlite_bedrock::request_policy::{
    BedrockRequestPolicy, Params as MetadataParams, RequestValue,
};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_http::error::HttpError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::Level;

use super::fixture::Fixture;
use super::input::{Constructor, ErrorClass, EventStream, Input, InvokeUrl, Transform};
use super::logs::capture_logs;

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockMessages".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn metadata_params(call: &InvokeUrl) -> MetadataParams {
    let mut entries: IndexMap<String, RequestValue> = IndexMap::new();
    for (name, value) in call.metadata.iter().flatten() {
        entries.insert(name.clone(), RequestValue::Text(value.clone()));
    }
    if let Some(spend) = &call.spend_logs_metadata {
        let pairs = spend
            .iter()
            .map(|(name, value)| (name.clone(), RequestValue::Text(value.clone())))
            .collect();
        entries.insert(
            "spend_logs_metadata".to_owned(),
            RequestValue::Mapping(pairs),
        );
    }
    let mut params = MetadataParams::new();
    if !entries.is_empty() {
        params.insert("metadata".to_owned(), RequestValue::Mapping(entries));
    }
    params
}

fn environment<'call>(call: &'call InvokeUrl) -> InvokeEnvironment<'call> {
    InvokeEnvironment {
        region_name: call.environment.get("AWS_REGION_NAME").map(String::as_str),
        region: call.environment.get("AWS_REGION").map(String::as_str),
        session_region: None,
        runtime_endpoint: call
            .environment
            .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
            .map(String::as_str),
    }
}

fn signed_headers(pairs: &[(String, String)]) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    for (name, value) in pairs {
        object.insert(name.clone(), Value::String(value.clone()));
    }
    object
}

fn request_policy(call: &InvokeUrl) -> Option<BedrockRequestPolicy> {
    let fields: IndexSet<String> = call.policy_fields.iter().flatten().cloned().collect();
    (!fields.is_empty()).then(|| BedrockRequestPolicy { fields })
}

fn messages_environment(
    call: &InvokeUrl,
) -> Result<(IndexMap<String, String>, Option<String>), Box<HelperError>> {
    let metadata = metadata_params(call);
    validate_messages_environment(
        &call.headers,
        &metadata,
        request_policy(call).as_ref(),
        call.api_base.as_deref(),
        call.api_key.as_ref(),
    )
    .map_err(failure)
}

fn complete_url(call: &InvokeUrl) -> Result<InvokeModelUrl, Box<HelperError>> {
    messages_complete_url(MessagesEndpoint {
        api_base: call.api_base.as_deref(),
        model: &call.model,
        optional_params: &call.optional_params,
        stream: call.stream,
        environment: environment(call),
    })
    .map_err(failure)
}

fn fixture_clock(now: i64) -> Result<SystemTime, Box<HelperError>> {
    let seconds = u64::try_from(now).map_err(failure)?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| failure("the fixture clock overflows"))
}

fn signed_request(
    call: &InvokeUrl,
    url: &InvokeModelUrl,
    headers: &IndexMap<String, String>,
) -> Result<AwsSignedRequest, Box<HelperError>> {
    let credentials = Credentials {
        access_key_id: call.credentials.access_key_id.clone(),
        secret_access_key: call.credentials.secret_access_key.clone(),
        session_token: call.credentials.session_token.clone(),
    };
    let request = MessagesSignRequest {
        headers,
        optional_params: &call.optional_params,
        request_data: &call.request_data,
        api_base: &url.endpoint_url,
        api_key: call.api_key.as_ref(),
        configured_bearer: call.configured_bearer_token.as_ref(),
        model: Some(&call.model),
        stream: call.stream,
        fake_stream: call.fake_stream,
        // Python signs inside signing_harness, which seals the environment, so only
        // optional_params can pick the region on that side.
        region_inputs: RegionInputs::default(),
        now: fixture_clock(call.now)?,
    };
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    runtime
        .block_on(sign_messages_request(request, move |_settings, _region| {
            std::future::ready(Ok::<_, CredentialError>(credentials))
        }))
        .map_err(failure)
}

fn header_object(headers: &IndexMap<String, String>) -> serde_json::Map<String, Value> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect()
}

fn invoke_url(call: &InvokeUrl) -> HelperResult {
    let (headers, unchanged) = messages_environment(call)?;
    let url = complete_url(call)?;
    let signed = signed_request(call, &url, &headers)?;
    Ok(json!({
        "url": {
            "endpoint_url": url.endpoint_url,
        },
        "environment": {"headers": header_object(&headers), "api_base": unchanged},
        "signed": {
            "method": signed.method,
            "url": signed.url,
            "headers": signed_headers(&signed.headers),
            "body": STANDARD.encode(signed.body),
        },
    }))
}

// The same prefix filter the Python helper applies, so unrelated warnings never reach the answer.
fn kept_warnings(messages: Vec<String>) -> Vec<String> {
    messages
        .into_iter()
        .filter(|message| {
            message.starts_with("Dropping ") || message.starts_with("Bedrock Invoke: ")
        })
        .collect()
}

fn transform(call: &Transform) -> HelperResult {
    let fixture = Fixture::load(&call.model_cost, call.model_info.clone()).map_err(failure)?;
    let (_guard, logs) = capture_logs();
    let mut answers = Vec::with_capacity(call.calls.len());
    for entry in &call.calls {
        let body = fixture
            .transform(entry, call.drop_params)
            .map_err(failure)?;
        answers.push(json!({
            "body": body,
            "warnings": kept_warnings(logs.take(Level::WARN)),
        }));
    }
    Ok(json!({"calls": answers}))
}

fn event_stream(call: &EventStream) -> HelperResult {
    let chunks = call
        .chunks
        .iter()
        .map(|chunk| hex::decode(chunk).map_err(failure))
        .collect::<Result<Vec<_>, _>>()?;
    let chunks =
        futures_util::stream::iter(chunks.into_iter().map(Ok::<Vec<u8>, HttpError>)).boxed();
    let redactor = Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?);
    let mut stream = messages_sse_stream(
        MessagesStreamRequest {
            response_headers: call.response_headers.clone(),
            redactor,
        },
        chunks,
    );
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let answer = runtime.block_on(async {
        let mut pieces = Vec::new();
        let mut failure = Value::Null;
        loop {
            match stream.next_chunk().await {
                Ok(Some(piece)) => pieces.push(hex::encode(piece)),
                Ok(None) => break,
                Err(error) => {
                    failure = json!({
                        "class": error.kind(),
                        "message": error.message(),
                        "status": error.status(),
                    });
                    break;
                }
            }
        }
        drop(stream.aclose().await);
        json!({"chunks": pieces, "failure": failure})
    });
    Ok(answer)
}

fn error_class(call: &ErrorClass) -> HelperResult {
    let headers: Vec<(String, String)> = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = get_error_class(&call.message, call.status_code, &headers).map_err(failure)?;
    let projected: serde_json::Map<String, Value> = match &error.headers {
        Some(ErrorHeaders::Http(pairs)) => pairs
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect(),
        Some(ErrorHeaders::Mapping(_)) | None => serde_json::Map::new(),
    };
    Ok(json!({
        "status": error.error.status_code,
        "message": error.error.message,
        "headers": projected,
    }))
}

fn constructor(call: &Constructor) -> HelperResult {
    MessagesConfig::try_from(&call.kwargs)
        .map(|_| json!({"created": true}))
        .map_err(|error| {
            Box::new(HelperError {
                class: "TypeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::InvokeUrl(call) => invoke_url(call),
        Input::Transform(call) => transform(call),
        Input::EventStream(call) => event_stream(call),
        Input::ErrorClass(call) => error_class(call),
        Input::Constructor(call) => constructor(call),
    }
}
