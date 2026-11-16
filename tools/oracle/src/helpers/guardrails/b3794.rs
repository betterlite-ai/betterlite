use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::slice;
use std::sync::{Arc, Mutex};
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
use betterlite_guardrails::repello_lifecycle::hooks::{HookContext, pre_call_hook};
use betterlite_guardrails::repello_lifecycle::initialize::{
    CallbackRegistry, initialize_guardrail,
};
use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};
use tracing::Level;
use tracing_subscriber::fmt;

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
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    unreachable_fallback: Option<String>,
    #[serde(default)]
    data: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
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
    content_type: Option<String>,
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
    event_hooks: Vec<&'static str>,
    requests: Vec<RequestRecord>,
    applied: Vec<String>,
    status: Option<String>,
    caught: Option<Caught>,
    warned: bool,
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
        unreachable_fallback: match call.unreachable_fallback.as_deref() {
            Some("fail_open") => UnreachableFallback::FailOpen,
            _ => UnreachableFallback::FailClosed,
        },
        timeout: None,
        mode: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: true,
        strict_modes: true,
    }
}

fn init_output(call: &Call, params: &RepelloAIParams) -> Output {
    let event_hooks = event_hooks();
    let Ok(settings) = RepelloAISettings::resolve(params, &call.env) else {
        return Output {
            event_hooks,
            ..Output::default()
        };
    };
    let key_source = key_source(&settings, call);
    Output {
        created: true,
        api_base: Some(settings.api_base),
        asset_id: Some(settings.asset_id),
        key_source: Some(key_source),
        event_hooks,
        ..Output::default()
    }
}

fn key_source(settings: &RepelloAISettings, call: &Call) -> &'static str {
    let key = settings.api_key.expose_secret();
    if call
        .api_key
        .as_deref()
        .is_some_and(|given| !given.is_empty() && given == key)
    {
        return "explicit";
    }
    if call
        .env
        .get("ARGUS_API_KEY")
        .is_some_and(|value| !value.is_empty() && value == key)
    {
        return "argus";
    }
    "repelloai"
}

fn hook_output(call: &Call, params: &RepelloAIParams) -> Result<Output, Box<HelperError>> {
    let resolved = RepelloAISettings::resolve(params, &call.env)
        .map_err(|error| failure("RepelloAIGuardrailMissingSecrets", error.to_string()))?;
    let mock = MockProvider::start(script(call)?)
        .map_err(|error| failure("MockError", error.to_string()))?;
    let guardrail = build_guardrail(params, &mock.base_url, &call.env)?;
    let captured = CapturedLog::default();
    let _log_guard = tracing::subscriber::set_default(captured.subscriber());
    let ran = build_runtime()?.block_on(run_pre_call(&guardrail, call.data.clone()))?;
    let key_source = key_source(&guardrail.settings, call);
    Ok(Output {
        created: true,
        api_base: Some(resolved.api_base),
        asset_id: Some(guardrail.settings.asset_id.clone()),
        key_source: Some(key_source),
        event_hooks: event_hooks(),
        requests: request_records(mock)?,
        applied: applied(&ran.data),
        status: recorded_status(&ran.data),
        caught: caught(&ran.result),
        warned: !captured.text().is_empty(),
    })
}

struct NoRegistry;

impl CallbackRegistry for NoRegistry {
    fn add_litellm_callback(&mut self, _guardrail: Arc<RepelloAILifecycle>) {}
}

fn build_guardrail(
    params: &RepelloAIParams,
    base_url: &str,
    env: &BTreeMap<String, String>,
) -> Result<Arc<RepelloAILifecycle>, Box<HelperError>> {
    let mut pointed = params.clone();
    pointed.api_base = Some(base_url.to_owned());
    let mut registry = NoRegistry;
    initialize_guardrail(&pointed, env, base_builder(), &mut registry)
        .map_err(|error| failure("ValueError", error.to_string()))
}

struct Ran {
    result: Result<(), GuardrailFailure>,
    data: BTreeMap<String, GuardrailValue>,
}

async fn run_pre_call(
    guardrail: &RepelloAILifecycle,
    mut data: BTreeMap<String, GuardrailValue>,
) -> Result<Ran, Box<HelperError>> {
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
    let result = pre_call_hook(guardrail, &mut data, &mut context).await;
    Ok(Ran { result, data })
}

struct NoSink;

impl GuardrailSpanSink for NoSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn caught(result: &Result<(), GuardrailFailure>) -> Option<Caught> {
    match result {
        Ok(()) => None,
        Err(GuardrailFailure::Http { status, detail }) => Some(Caught {
            status: Some(*status),
            detail: Value::String(detail.clone()),
        }),
        Err(GuardrailFailure::HttpObject { status, detail }) => Some(Caught {
            status: Some(*status),
            detail: serde_json::to_value(detail).unwrap_or(Value::Null),
        }),
        Err(other) => Some(Caught {
            status: None,
            detail: Value::String(other.to_string()),
        }),
    }
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
            content_type: call.request.headers.get("content-type").cloned(),
            body: call.request.body,
        })
        .collect())
}

fn event_hooks() -> Vec<&'static str> {
    vec!["pre_call", "post_call"]
}

fn bucket(data: &BTreeMap<String, GuardrailValue>) -> Option<&BTreeMap<String, GuardrailValue>> {
    let value = data
        .get("litellm_metadata")
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .or_else(|| data.get("metadata"))?;
    match value {
        GuardrailValue::Object(fields) => Some(fields),
        _ => None,
    }
}

fn applied(data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    let Some(GuardrailValue::Array(names)) =
        bucket(data).and_then(|fields| fields.get("applied_guardrails"))
    else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|name| match name {
            GuardrailValue::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn recorded_status(data: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    let GuardrailValue::Array(records) =
        bucket(data)?.get("standard_logging_guardrail_information")?
    else {
        return None;
    };
    let GuardrailValue::Object(record) = records.last()? else {
        return None;
    };
    match record.get("guardrail_status") {
        Some(GuardrailValue::Text(status)) => Some(status.clone()),
        _ => None,
    }
}

fn script(call: &Call) -> Result<MockScript, Box<HelperError>> {
    let answer = call
        .answer
        .as_ref()
        .ok_or_else(|| failure("MockError", "the case has no answer".to_owned()))?;
    MockScript::try_from(slice::from_ref(answer))
        .map_err(|error| failure("MockError", error.to_string()))
}

fn build_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
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

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    fn text(&self) -> String {
        let bytes = self.0.lock().map(|log| log.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn subscriber(&self) -> impl tracing::Subscriber + use<> {
        let writer = self.clone();
        fmt()
            .with_max_level(Level::WARN)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish()
    }
}

impl io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the captured log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
