use std::collections::BTreeMap;
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use betterlite_bedrock::signing::body::Body;
use betterlite_bedrock::signing::endpoint::{RequestHeaders as EndpointRequest, request_headers};
use betterlite_bedrock::signing::executor::{AwsSigningExecutor, sign_request_off_loop_if_aws};
use betterlite_bedrock::signing::headers::filter_for_signature;
use betterlite_bedrock::signing::json_post::{JsonPost, sign_aws_json_post};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::service::{ServiceRequest, sign_service_request};
use betterlite_bedrock::signing::target::{RegionInputs, TargetRequest, request_target};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::with_default;
use tracing_subscriber::registry;

use super::input::{
    BatchDataInput, BatchRequest as BatchRequestInput, Call, CredentialInput, DataInput,
    JsonPost as JsonPostInput, OffLoop, RequestHeaders as RequestHeadersInput,
    RequestTarget as RequestTargetInput, ServiceRequest as ServiceRequestInput,
};
use crate::helpers::HelperError;

pub(super) fn run_call(call: &Call) -> Value {
    match call {
        Call::RequestHeaders(case) => step(request_headers_case(case)),
        Call::FilterHeaders(case) => json!({
            "ok": true,
            "headers": headers_json(&filter_for_signature(&optional_headers(&case.headers))),
        }),
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
            bytes = STANDARD
                .decode(encoded)
                .map_err(|error| input_error(error.to_string()))?;
            Body::Bytes(&bytes)
        }
    };
    let headers = optional_headers(&case.headers);
    let extra = resolved_headers(&case.extra_headers);
    let api_key = case
        .api_key
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let configured = case
        .configured_bearer_token
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let request = EndpointRequest {
        credentials: credentials.as_ref(),
        region: &case.region,
        extra_headers: &extra,
        endpoint: &case.endpoint,
        data,
        headers: &headers,
        api_key: api_key.as_ref(),
        configured_bearer: configured.as_ref(),
        supports_bearer_token: case.supports_bearer_token,
        now: epoch(case.now)?,
    };
    Ok(match request_headers(request) {
        Ok(signed) => signed_json(&signed),
        Err(_error) => json!({"ok": false, "status": null}),
    })
}

fn service_request_case(case: &ServiceRequestInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let headers = optional_headers(&case.headers);
    let api_key = case
        .api_key
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let configured = case
        .configured_bearer_token
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let regions = region_inputs(&case.environment);
    let now = epoch(case.now)?;
    let runtime = runtime()?;
    let result = runtime.block_on(sign_service_request(
        ServiceRequest {
            service: case.service_name,
            headers: &headers,
            optional_params: &case.optional_params,
            request_data: &case.request_data,
            api_base: &case.api_base,
            model: case.model.as_deref(),
            api_key: api_key.as_ref(),
            configured_bearer: configured.as_ref(),
            region_inputs: regions,
            now,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture.clone())),
    ));
    Ok(match result {
        Ok(signed) => signed_json(&signed),
        Err(_error) => json!({"ok": false, "status": null}),
    })
}

fn json_post_case(case: &JsonPostInput) -> Result<Value, Box<HelperError>> {
    let credentials = credentials_of(&case.credentials);
    let headers = resolved_headers(&case.headers);
    let result = sign_aws_json_post(JsonPost {
        service_name: &case.service_name,
        region: case.region.as_deref(),
        url: &case.url,
        body: &case.body,
        headers: &headers,
        credentials: &credentials,
        now: epoch(case.now)?,
    });
    Ok(match result {
        Ok(signed) => signed_json(&signed),
        Err(_error) => json!({"ok": false, "status": null}),
    })
}

fn request_target_case(case: &RequestTargetInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let bearer = case
        .bearer_token
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let mut parameters = case.optional_params.clone();
    let resolved = AtomicUsize::new(0);
    let runtime = runtime()?;
    let result = runtime.block_on(request_target(
        TargetRequest {
            optional_params: &mut parameters,
            model: case.model.as_deref(),
            region_inputs: region_inputs(&case.environment),
            bearer_token: bearer.as_ref(),
        },
        |_settings, _region| {
            resolved.fetch_add(1, Ordering::Relaxed);
            ready(Ok::<Credentials, CredentialError>(fixture.clone()))
        },
    ));
    match result {
        Ok(target) => {
            let remaining = serde_json::to_value(&parameters)
                .map_err(|error| input_error(error.to_string()))?;
            Ok(json!({
                "ok": true,
                "region": target.region,
                "endpoint": target.endpoint,
                "credentials": target.credentials.as_ref().map(credentials_json),
                "remaining": remaining,
                "resolved": resolved.load(Ordering::Relaxed),
            }))
        }
        Err(_error) => Ok(json!({"ok": false, "status": null})),
    }
}

fn batch_request_case(case: &BatchRequestInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let data = match &case.data {
        BatchDataInput::Json(object) => BatchData::Json(object),
        BatchDataInput::Text(text) => BatchData::Text(text),
    };
    let now = epoch(case.now)?;
    let runtime = runtime()?;
    let result = runtime.block_on(sign_batch_request(
        BatchRequest {
            service_name: &case.service_name,
            data,
            endpoint: &case.endpoint_url,
            optional_params: &case.optional_params,
            method: case.method.as_deref(),
            region_inputs: region_inputs(&case.environment),
            now,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture.clone())),
    ));
    Ok(match result {
        Ok(signed) => signed_json(&signed),
        Err(_error) => json!({"ok": false, "status": null}),
    })
}

fn off_loop_case(case: &OffLoop) -> Result<Value, Box<HelperError>> {
    let runtime = runtime()?;
    with_default(registry(), || {
        let span = tracing::info_span!("bedrock_signing", context = %case.context);
        let entered = span.enter();
        let probe = Probe {
            value: case.value.clone(),
            context: case.context.clone(),
            span_id: span.id(),
            caller: std::thread::current().id(),
        };
        let steps = runtime.block_on(observed_steps(&probe, &case.failure));
        drop(entered);
        Ok(json!({"ok": true, "steps": steps}))
    })
}

async fn observed_steps(probe: &Probe, failure_message: &str) -> Vec<Value> {
    let executor = AwsSigningExecutor::default();
    let mut steps = Vec::new();
    let run_probe = probe.clone();
    steps.push(match executor.run(move || run_probe.observe()).await {
        Ok(observed) => value_json("run", &observed),
        Err(_error) => json!({"ok": false, "status": null}),
    });
    let inline_probe = probe.clone();
    steps.push(
        match sign_request_off_loop_if_aws(&executor, false, move || inline_probe.observe()).await {
            Ok(observed) => value_json("inline", &observed),
            Err(_error) => json!({"ok": false, "status": null}),
        },
    );
    let offloaded_probe = probe.clone();
    steps.push(
        match sign_request_off_loop_if_aws(&executor, true, move || offloaded_probe.observe()).await
        {
            Ok(observed) => value_json("offloaded", &observed),
            Err(_error) => json!({"ok": false, "status": null}),
        },
    );
    let failure_text = failure_message.to_owned();
    steps.push(
        match executor
            .run(move || Err::<Observed, String>(failure_text))
            .await
        {
            Ok(Ok(observed)) => value_json("failure", &observed),
            Ok(Err(message)) => json!({"kind": "failure", "failed": true, "value": message}),
            Err(_error) => json!({"ok": false, "status": null}),
        },
    );
    steps
}

#[derive(Clone)]
struct Probe {
    value: String,
    context: String,
    span_id: Option<tracing::Id>,
    caller: ThreadId,
}

impl Probe {
    fn observe(&self) -> Observed {
        Observed {
            value: self.value.clone(),
            context: self.context.clone(),
            context_preserved: tracing::Span::current().id() == self.span_id,
            off_caller_thread: std::thread::current().id() != self.caller,
        }
    }
}

struct Observed {
    value: String,
    context: String,
    context_preserved: bool,
    off_caller_thread: bool,
}

fn value_json(kind: &str, observed: &Observed) -> Value {
    json!({
        "kind": kind,
        "failed": false,
        "value": observed.value,
        "context": observed.context,
        "context_preserved": observed.context_preserved,
        "off_caller_thread": observed.off_caller_thread,
    })
}

fn signed_json(signed: &AwsSignedRequest) -> Value {
    json!({
        "ok": true,
        "method": &signed.method,
        "url": &signed.url,
        "headers": headers_json(&signed.headers),
        "body": STANDARD.encode(&signed.body),
    })
}

fn headers_json(headers: &[(String, String)]) -> Value {
    let entries: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    Value::Object(entries)
}

fn credentials_of(input: &CredentialInput) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(input.access_key_id.clone()),
        secret_access_key: SecretString::from(input.secret_access_key.clone()),
        session_token: input
            .session_token
            .as_deref()
            .map(|token| SecretString::from(token.to_owned())),
    }
}

fn credentials_json(credentials: &Credentials) -> Value {
    json!([
        credentials.access_key_id.expose_secret(),
        credentials.secret_access_key.expose_secret(),
        credentials
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
    ])
}

fn optional_headers(headers: &BTreeMap<String, Option<String>>) -> Vec<(String, Option<String>)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn resolved_headers(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn region_inputs(environment: &BTreeMap<String, String>) -> RegionInputs<'_> {
    RegionInputs {
        environment_region_name: environment.get("AWS_REGION_NAME").map(String::as_str),
        environment_region: environment.get("AWS_REGION").map(String::as_str),
        session_region: environment.get("AWS_DEFAULT_REGION").map(String::as_str),
    }
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds)
        .map_err(|_negative| input_error("now must not be negative".to_owned()))?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or_else(|| input_error("now is out of range".to_owned()))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| input_error(error.to_string()))
}

fn input_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
