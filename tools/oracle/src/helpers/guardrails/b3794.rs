use std::collections::{BTreeMap, BTreeSet};
use std::slice;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::invocation::{
    GuardrailHookName, GuardrailInvocation,
};
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::repello_lifecycle::RepelloAILifecycle;
use betterlite_guardrails::repello_lifecycle::config::{RepelloAIParams, RepelloAISettings};
use betterlite_guardrails::repello_lifecycle::hooks::{HookContext, HookTarget, scan_hook};
use betterlite_guardrails::repello_lifecycle::{CallbackRegistry, Data, initialize_guardrail};
use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    mode: Mode,
    #[serde(default)]
    env: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    asset_id: Option<String>,
    #[serde(default)]
    unreachable_fallback: UnreachableFallback,
    #[serde(default)]
    data: BTreeMap<String, GuardrailValue>,
    answer: Option<UpstreamAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Init,
    PreCall,
}

#[derive(Serialize)]
struct RequestRecord {
    path: String,
    api_key_present: bool,
    body: Value,
}

#[derive(Serialize)]
struct Caught {
    status: Option<u16>,
    detail: Value,
}

#[derive(Serialize, Default)]
struct Output {
    created: bool,
    api_base: Option<String>,
    asset_id: Option<String>,
    key_source: Option<&'static str>,
    requests: Vec<RequestRecord>,
    applied: Vec<String>,
    status: Option<String>,
    caught: Option<Caught>,
}

// The metadata the hook writes: the applied-guardrails header and every record, read typed.
#[derive(Deserialize, Default)]
struct Metadata {
    #[serde(default)]
    applied_guardrails: Vec<String>,
    #[serde(default, rename = "standard_logging_guardrail_information")]
    records: Vec<RecordedGuardrail>,
}

#[derive(Deserialize)]
struct RecordedGuardrail {
    guardrail_status: Option<String>,
}

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        run_call(input)
    }
}

fn run_call(call: &Call) -> HelperResult {
    let params = build_params(call);
    let output = match call.mode {
        Mode::Init => init_output(call, &params),
        Mode::PreCall => hook_output(call, &params)?,
    };
    serde_json::to_value(output).map_err(|error| failure("ValueError", error.to_string()))
}

fn build_params(call: &Call) -> RepelloAIParams {
    RepelloAIParams {
        guardrail_name: Some("repello".to_owned()),
        api_key: call.api_key.clone().map(SecretString::from),
        api_base: call.api_base.clone(),
        asset_id: call.asset_id.clone(),
        unreachable_fallback: call.unreachable_fallback,
        timeout: None,
        mode: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: true,
        strict_modes: true,
    }
}

fn init_output(call: &Call, params: &RepelloAIParams) -> Output {
    let mut registry = NoRegistry;
    match initialize_guardrail(params, &call.env, base_builder(), &mut registry) {
        Ok(guardrail) => base_output(call, &guardrail.settings),
        Err(_error) => Output::default(),
    }
}

// The settings as the guardrail resolved them, read before the mock base replaces api_base.
fn base_output(call: &Call, settings: &RepelloAISettings) -> Output {
    Output {
        created: true,
        api_base: Some(settings.api_base.clone()),
        asset_id: Some(settings.asset_id.clone()),
        key_source: Some(key_source(call, settings.api_key.expose_secret())),
        requests: Vec::new(),
        applied: Vec::new(),
        status: None,
        caught: None,
    }
}

fn key_source(call: &Call, key: &str) -> &'static str {
    if call.api_key.as_deref() == Some(key) {
        return "explicit";
    }
    if call.env.get("ARGUS_API_KEY").map(String::as_str) == Some(key) {
        return "argus";
    }
    "repelloai"
}

struct NoRegistry;

impl CallbackRegistry for NoRegistry {
    fn add_litellm_callback(&mut self, _guardrail: Arc<RepelloAILifecycle>) {}
}

fn hook_output(call: &Call, params: &RepelloAIParams) -> Result<Output, Box<HelperError>> {
    let settings = RepelloAISettings::resolve(params, &call.env)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let mock = MockProvider::start(script(call)?)
        .map_err(|error| failure("MockError", error.to_string()))?;
    let mut pointed = params.clone();
    pointed.api_base = Some(mock.base_url.clone());
    let mut registry = NoRegistry;
    let guardrail = initialize_guardrail(&pointed, &call.env, base_builder(), &mut registry)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    let (result, data) = runtime.block_on(run_pre_call(&guardrail, call.data.clone()))?;
    let meta = metadata(&data);
    Ok(Output {
        requests: request_records(mock)?,
        applied: meta.applied_guardrails,
        status: meta
            .records
            .last()
            .and_then(|record| record.guardrail_status.clone()),
        caught: caught(&result),
        ..base_output(call, &settings)
    })
}

async fn run_pre_call(
    guardrail: &RepelloAILifecycle,
    mut data: Data,
) -> Result<(Result<(), GuardrailFailure>, Data), Box<HelperError>> {
    let mut invocation =
        GuardrailInvocation::begin(GuardrailHookName::PreCall, &data, None, SystemTime::now())
            .map_err(|error| failure("RecordError", error.to_string()))?;
    let suppressed = BTreeSet::new();
    let sink = NoSink;
    let mut context = HookContext {
        suppressed: &suppressed,
        invocation: &mut invocation,
        sink: &sink,
        log_target: None,
    };
    let result = scan_hook(guardrail, &mut data, HookTarget::Prompt, &mut context).await;
    Ok((result, data))
}

struct NoSink;

impl GuardrailSpanSink for NoSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn caught(result: &Result<(), GuardrailFailure>) -> Option<Caught> {
    let failure = result.as_ref().err()?;
    let (status, detail) = match failure {
        GuardrailFailure::Http { status, detail } => (Some(*status), Value::String(detail.clone())),
        GuardrailFailure::HttpObject { status, detail } => (
            Some(*status),
            serde_json::to_value(detail).unwrap_or(Value::Null),
        ),
        GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_) => (None, Value::String(failure.to_string())),
    };
    Some(Caught { status, detail })
}

fn metadata(data: &BTreeMap<String, GuardrailValue>) -> Metadata {
    data.get("litellm_metadata")
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| data.get("metadata"))
        .and_then(|value| serde_json::to_value(value).ok())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn request_records(mock: MockProvider) -> Result<Vec<RequestRecord>, Box<HelperError>> {
    let received = mock
        .finish()
        .map_err(|error| failure("MockError", error.to_string()))?;
    Ok(received
        .into_iter()
        .map(|call| RequestRecord {
            path: call.request.path,
            api_key_present: call.request.headers.contains_key("x-api-key"),
            body: call.request.body,
        })
        .collect())
}

fn script(call: &Call) -> Result<MockScript, Box<HelperError>> {
    let answer = call
        .answer
        .as_ref()
        .ok_or_else(|| failure("MockError", "the case has no answer".to_owned()))?;
    MockScript::try_from(slice::from_ref(answer))
        .map_err(|error| failure("MockError", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
