use super::input::{
    BatchRequest as BatchRequestInput, OffLoop, RequestTarget as RequestTargetInput,
    ServiceRequest as ServiceRequestInput,
};
use super::ops::{credentials_json, credentials_of, epoch, runtime, signed_result};
use crate::helpers::{HelperError, OrHelperError as _};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::batch::sign_batch_request;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::executor::{AwsSigningExecutor, sign_request_off_loop_if_aws};
use betterlite_bedrock::signing::service::sign_service_request;
use betterlite_bedrock::signing::target::{RegionInputs, TargetRequest, request_target};
use serde_json::{Value, json};
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;
use tracing::subscriber::with_default;
use tracing_subscriber::registry;
pub(super) fn service_request_case(case: &ServiceRequestInput) -> Result<Value, Box<HelperError>> {
    let headers = case.headers.clone().into_iter().collect::<Vec<_>>();
    Ok(signed_result(runtime()?.block_on(sign_service_request(
        case.request(&headers, epoch(case.now)?),
        |_settings, _region| ready(Ok(credentials_of(&case.credentials))),
    ))))
}
pub(super) fn batch_request_case(case: &BatchRequestInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let signing = sign_batch_request(case.request(epoch(case.now)?), |_settings, _region| {
        ready(Ok::<Credentials, CredentialError>(fixture.clone()))
    });
    Ok(signed_result(runtime()?.block_on(signing)))
}
pub(super) fn request_target_case(case: &RequestTargetInput) -> Result<Value, Box<HelperError>> {
    let fixture = credentials_of(&case.credentials);
    let mut parameters = case.optional_params.clone();
    let resolved = AtomicUsize::new(0);
    let request = TargetRequest {
        optional_params: &mut parameters,
        model: case.model.as_deref(),
        region_inputs: RegionInputs::default(),
        bearer_token: case.bearer_token.as_ref(),
    };
    let resolving = request_target(request, |_settings, _region| {
        resolved.fetch_add(1, Ordering::Relaxed);
        ready(Ok::<Credentials, CredentialError>(fixture.clone()))
    });
    let Ok(target) = runtime()?.block_on(resolving) else {
        return Ok(json!({"ok": false, "status": null}));
    };
    let remaining = serde_json::to_value(&parameters).or_value_error()?;
    Ok(json!({
        "ok": true,
        "region": target.region,
        "endpoint": target.endpoint,
        "credentials": target.credentials.as_ref().map(credentials_json),
        "remaining": remaining,
        "resolved": resolved.load(Ordering::Relaxed),
    }))
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
    let observe = |kind: &'static str| {
        let owned = probe.clone();
        move || owned.observe(kind)
    };
    let failure = failure_message.to_owned();
    vec![
        executor.run(observe("run")).await.unwrap_or_else(unsigned),
        sign_request_off_loop_if_aws(&executor, false, observe("inline"))
            .await
            .unwrap_or_else(unsigned),
        sign_request_off_loop_if_aws(&executor, true, observe("offloaded"))
            .await
            .unwrap_or_else(unsigned),
        match executor.run(move || Err::<Value, String>(failure)).await {
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
