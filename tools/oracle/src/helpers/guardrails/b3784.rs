use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_guardrails::ovalix::environment;
use betterlite_guardrails::ovalix::error::OvalixError;
use betterlite_guardrails::ovalix::exception_status::{
    GuardrailRaisedException, OvalixBlockedException,
};
use betterlite_guardrails::ovalix::session_identity::{Clock, RequestData};
use betterlite_guardrails::ovalix::{CallHook, InputType, Inputs, OvalixGuardrail, Params};
use chrono::NaiveDate;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ProviderCall, ScriptedReply, Wait};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    OvalixCall(Call),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    request_data: RequestData,
    #[serde(default)]
    input_type: InputType,
    #[serde(default)]
    texts: Vec<String>,
    today: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    answers: Vec<UpstreamAnswer>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum Mode {
    One(CallHook),
    Many(Vec<CallHook>),
}

impl Default for Mode {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl Mode {
    fn hooks(&self) -> Vec<CallHook> {
        match self {
            Self::One(hook) => vec![*hook],
            Self::Many(hooks) => hooks.clone(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    tracker_api_base: Option<String>,
    tracker_api_key: Option<SecretString>,
    application_id: Option<String>,
    pre_checkpoint_id: Option<String>,
    post_checkpoint_id: Option<String>,
}

struct FixedClock(NaiveDate);

impl Clock for FixedClock {
    fn today(&self) -> NaiveDate {
        self.0
    }
}

struct Prepared {
    mock: MockProvider,
    guardrail: OvalixGuardrail<FixedClock>,
    tracker_api_base: String,
    event_hooks: Vec<CallHook>,
}

#[derive(Serialize)]
struct RequestRecord {
    path: String,
    body: Value,
}

#[derive(Serialize)]
struct Output {
    blocked: bool,
    status: Option<u16>,
    tracker_api_base: String,
    event_hooks: Vec<CallHook>,
    requests: Vec<RequestRecord>,
    texts: Option<Vec<String>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Input::OvalixCall(call) = input;
        run_call(call)
    }
}

fn run_call(call: &Call) -> HelperResult {
    let prepared = prepare(call)?;
    let result = build_runtime()?.block_on(prepared.guardrail.apply_guardrail(
        Inputs {
            texts: call.texts.clone(),
            images: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
            model: None,
            stream_holdback_chars: None,
        },
        &call.request_data,
        call.input_type,
    ));
    let requests = record_requests(
        prepared
            .mock
            .finish()
            .map_err(|error| failure("ValueError", None, error.to_string()))?,
    );
    let (blocked, status, texts) = match &result {
        Ok(output) => (false, None, Some(output.texts.clone())),
        Err(error) => {
            let (blocked, status) = ovalix_failure(error);
            (blocked, Some(status), None)
        }
    };
    serde_json::to_value(Output {
        blocked,
        status,
        tracker_api_base: prepared.tracker_api_base,
        event_hooks: prepared.event_hooks,
        requests,
        texts,
    })
    .map_err(|error| failure("ValueError", None, error.to_string()))
}

fn prepare(call: &Call) -> Result<Prepared, Box<HelperError>> {
    let today = NaiveDate::parse_from_str(&call.today, "%Y-%m-%d")
        .map_err(|error| failure("ValueError", None, error.to_string()))?;
    let params = Params {
        guardrail_name: Some("ovalix".to_owned()),
        tracker_api_base: call.settings.tracker_api_base.clone(),
        tracker_api_key: call.settings.tracker_api_key.clone(),
        application_id: call.settings.application_id.clone(),
        pre_checkpoint_id: call.settings.pre_checkpoint_id.clone(),
        post_checkpoint_id: call.settings.post_checkpoint_id.clone(),
        event_hooks: Vec::new(),
        mode: call.mode.hooks(),
        default_on: false,
        timeout: None,
    };
    let resolved = environment::resolve(&params, &call.env, &params.event_hooks)
        .map_err(|error| failure("OvalixGuardrailMissingSecrets", None, error.to_string()))?;
    let tracker_api_base = resolved.settings.tracker_api_base.clone();
    let mock = MockProvider::start(script(call)?)
        .map_err(|error| failure("ValueError", None, error.to_string()))?;
    let mut settings = resolved.settings;
    settings.tracker_api_base.clone_from(&mock.base_url);
    let guardrail = OvalixGuardrail {
        settings,
        guardrail_name: "ovalix".to_owned(),
        default_on: false,
        event_hooks: resolved.event_hooks.clone(),
        mode: params.mode,
        timeout: Duration::from_millis(call.timeout_ms.unwrap_or(1000)),
        client: reqwest::Client::default(),
        clock: FixedClock(today),
    };
    Ok(Prepared {
        mock,
        guardrail,
        tracker_api_base,
        event_hooks: resolved.event_hooks,
    })
}

fn script(call: &Call) -> Result<MockScript, Box<HelperError>> {
    if call.answers.is_empty() {
        return Ok(MockScript {
            deadline: Duration::from_secs(5),
            replies: vec![ScriptedReply {
                wait: Wait::Delay { millis: 500 },
                then: Outcome::Answer(UpstreamAnswer {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: Some(serde_json::json!({"action_type": "none"})),
                    events: None,
                }),
            }],
            barriers: BTreeMap::new(),
        });
    }
    MockScript::try_from(call.answers.as_slice())
        .map_err(|error| failure("ValueError", None, error.to_string()))
}

fn build_runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", None, error.to_string()))
}

fn record_requests(calls: Vec<ProviderCall>) -> Vec<RequestRecord> {
    calls
        .into_iter()
        .map(|received| RequestRecord {
            path: received.request.path,
            body: received.request.body,
        })
        .collect()
}

const fn ovalix_failure(error: &OvalixError) -> (bool, u16) {
    match error {
        OvalixError::Blocked(_) => (
            OvalixBlockedException::BLOCKED_CONTENT,
            OvalixBlockedException::STATUS_CODE,
        ),
        OvalixError::Raised(_) => (
            GuardrailRaisedException::BLOCKED_CONTENT,
            GuardrailRaisedException::STATUS_CODE,
        ),
    }
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
