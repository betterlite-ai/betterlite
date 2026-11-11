use super::input::{
    BatchDataInput, BatchRequest as BatchRequestInput, OffLoop,
    RequestTarget as RequestTargetInput, ServiceRequest as ServiceRequestInput,
};
use super::ops::{credentials_json, credentials_of, epoch, input_error, runtime, signed_result};
use crate::helpers::HelperError;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::batch::{BatchData, BatchRequest, sign_batch_request};
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::executor::{AwsSigningExecutor, sign_request_off_loop_if_aws};
use betterlite_bedrock::signing::service::{ServiceRequest, sign_service_request};
use betterlite_bedrock::signing::target::{RegionInputs, TargetRequest, request_target};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;
use tracing::subscriber::with_default;
use tracing_subscriber::registry;
pub(super) fn service_request_case(case: &ServiceRequestInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let headers = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    let api_key = case
        .api_key
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let configured = case
        .configured_bearer_token
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
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
            region_inputs: RegionInputs::default(),
            now,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture.clone())),
    ));
    Ok(signed_result(result))
}
pub(super) fn batch_request_case(case: &BatchRequestInput) -> Result<Value, Box<HelperError>> {
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
            region_inputs: RegionInputs::default(),
            now,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture.clone())),
    ));
    Ok(signed_result(result))
}
pub(super) fn request_target_case(case: &RequestTargetInput) -> Result<Value, Box<HelperError>> {
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
            region_inputs: RegionInputs::default(),
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
pub(super) fn off_loop_case(case: &OffLoop) -> Result<Value, Box<HelperError>> {
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
    let unsigned = |_error: SigningError| json!({"ok": false, "status": null});
    let run_probe = probe.clone();
    let inline_probe = probe.clone();
    let offloaded_probe = probe.clone();
    let failure_text = failure_message.to_owned();
    vec![
        executor
            .run(move || run_probe.observe("run"))
            .await
            .unwrap_or_else(unsigned),
        sign_request_off_loop_if_aws(&executor, false, move || inline_probe.observe("inline"))
            .await
            .unwrap_or_else(unsigned),
        sign_request_off_loop_if_aws(&executor, true, move || {
            offloaded_probe.observe("offloaded")
        })
        .await
        .unwrap_or_else(unsigned),
        match executor
            .run(move || Err::<Value, String>(failure_text))
            .await
        {
            Ok(Ok(value)) => value,
            Ok(Err(message)) => json!({"kind": "failure", "failed": true, "value": message}),
            Err(error) => unsigned(error),
        },
    ]
}
#[derive(Clone)]
struct Probe {
    value: String,
    context: String,
    span_id: Option<tracing::Id>,
    caller: ThreadId,
}
impl Probe {
    fn observe(&self, kind: &str) -> Value {
        json!({
            "kind": kind,
            "failed": false,
            "value": &self.value,
            "context": self.context,
            "context_preserved": tracing::Span::current().id() == self.span_id,
            "off_caller_thread": std::thread::current().id() != self.caller,
        })
    }
}
