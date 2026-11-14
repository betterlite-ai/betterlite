#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::{GuardrailFailure, SensitiveDataRouteFailure};
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_deployment::hook_target::{
    DeploymentHookTarget, HookTarget, PreCallOutcome, StampedKeyFields,
};
use crate::guardrail_deployment::hooks::{DeploymentHookError, PreCallContext};
use crate::guardrail_deployment::logging_hook::{LoggedCall, LoggedResultKind, LoggingHookError};
use crate::guardrail_deployment::markers::PreCallToken;
use crate::guardrail_deployment::messages_hook::MessagesHookError;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::guardrail_translation::{BaseTranslation, TranslationCall};
use crate::unified_dispatch::registry::{TranslationEntry, TranslationRegistry};

pub(super) type Data = BTreeMap<String, GuardrailValue>;

pub(super) fn data(json: &str) -> Data {
    serde_json::from_str(json).unwrap()
}

pub(super) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(super) fn token() -> PreCallToken {
    PreCallToken("tok".to_owned())
}

pub(super) fn guardrail(name: Option<&str>) -> GuardrailBase {
    let mut base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    base.settings.guardrail_name = name.map(str::to_owned);
    base
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

pub(super) fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    (set_default(subscriber), captured)
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PreCallSeen {
    pub(super) target: HookTarget,
    pub(super) key: StampedKeyFields,
    pub(super) data: Data,
    pub(super) call_type: CallType,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PostCallSeen {
    pub(super) target: HookTarget,
    pub(super) key: StampedKeyFields,
    pub(super) data: Data,
    pub(super) response: GuardrailValue,
}

// A hook target that answers what it was given and remembers every call
pub(super) struct Recorder {
    pub(super) pre: Result<PreCallOutcome, GuardrailFailure>,
    pub(super) post: Result<Option<GuardrailValue>, GuardrailFailure>,
    pub(super) pre_calls: Mutex<Vec<PreCallSeen>>,
    pub(super) post_calls: Mutex<Vec<PostCallSeen>>,
}

impl Recorder {
    pub(super) fn answering(pre: PreCallOutcome) -> Self {
        Self {
            pre: Ok(pre),
            post: Ok(None),
            pre_calls: Mutex::new(Vec::new()),
            post_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn post_answering(post: Result<Option<GuardrailValue>, GuardrailFailure>) -> Self {
        Self {
            pre: Ok(PreCallOutcome::Unchanged),
            post,
            pre_calls: Mutex::new(Vec::new()),
            post_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn pre_calls(&self) -> Vec<PreCallSeen> {
        self.pre_calls.lock().unwrap().clone()
    }

    pub(super) fn post_calls(&self) -> Vec<PostCallSeen> {
        self.post_calls.lock().unwrap().clone()
    }
}

impl DeploymentHookTarget for Recorder {
    async fn pre_call(
        &self,
        target: HookTarget,
        key: &StampedKeyFields,
        data: &mut Data,
        call_type: CallType,
    ) -> Result<PreCallOutcome, GuardrailFailure> {
        self.pre_calls.lock().unwrap().push(PreCallSeen {
            target,
            key: key.clone(),
            data: data.clone(),
            call_type,
        });
        self.pre.clone()
    }

    async fn post_call_success(
        &self,
        target: HookTarget,
        key: &StampedKeyFields,
        data: &mut Data,
        response: GuardrailValue,
    ) -> Result<Option<GuardrailValue>, GuardrailFailure> {
        self.post_calls.lock().unwrap().push(PostCallSeen {
            target,
            key: key.clone(),
            data: data.clone(),
            response,
        });
        self.post.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Seen {
    pub(super) label: &'static str,
    pub(super) phase: &'static str,
    pub(super) request: Data,
    pub(super) response: Option<GuardrailValue>,
}

// A translation that remembers what it scanned and records a fixed verdict into the request metadata
#[derive(Clone)]
pub(super) struct Scanner {
    pub(super) label: &'static str,
    pub(super) seen: Arc<Mutex<Vec<Seen>>>,
    pub(super) verdict: Option<GuardrailValue>,
    pub(super) fail_input: bool,
    pub(super) fail_output: bool,
}

impl Scanner {
    pub(super) fn new(label: &'static str, seen: &Arc<Mutex<Vec<Seen>>>) -> Self {
        Self {
            label,
            seen: Arc::clone(seen),
            verdict: None,
            fail_input: false,
            fail_output: false,
        }
    }

    fn record(&self, phase: &'static str, request: &Data, response: Option<GuardrailValue>) {
        self.seen.lock().unwrap().push(Seen {
            label: self.label,
            phase,
            request: request.clone(),
            response,
        });
    }

    fn write_verdict(&self, request: &mut Data) {
        let Some(verdict) = &self.verdict else {
            return;
        };
        if let Some(GuardrailValue::Object(metadata)) = request.get_mut("metadata") {
            metadata.insert(
                "standard_logging_guardrail_information".to_owned(),
                verdict.clone(),
            );
        }
    }
}

impl BaseTranslation for Scanner {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = io::Error;

    async fn process_input_messages(
        &self,
        data: &mut Data,
        _guardrail: &(),
        _logging_obj: Option<&mut ()>,
    ) -> Result<(), io::Error> {
        self.record("input", data, None);
        self.write_verdict(data);
        if self.fail_input {
            return Err(io::Error::other("input scan failed"));
        }
        Ok(())
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &(),
        call: TranslationCall<'_, ()>,
    ) -> Result<GuardrailValue, io::Error> {
        let Some(request) = call.request_data else {
            return Err(io::Error::other("no request data"));
        };
        self.record("output", request, Some(response.clone()));
        if self.fail_output {
            return Err(io::Error::other("output scan failed"));
        }
        Ok(response)
    }
}

pub(super) fn registry(entries: &[(CallType, Scanner)]) -> TranslationRegistry<Scanner> {
    TranslationRegistry {
        entries: entries
            .iter()
            .map(|(call_type, scanner)| TranslationEntry {
                call_type: *call_type,
                translation: scanner.clone(),
                chat_completions: false,
            })
            .collect(),
    }
}

pub(super) fn boom() -> GuardrailFailure {
    GuardrailFailure::Other("boom".to_owned())
}

pub(super) fn applying(name: Option<&str>) -> GuardrailBase {
    let mut base = guardrail(name);
    base.settings.switches.uses_apply_guardrail_interface = true;
    base
}

pub(super) fn with_hook(mut base: GuardrailBase, hook: &str) -> GuardrailBase {
    base.settings.event_hook = Some(LitellmParamsMode::One(hook.to_owned()));
    base
}

pub(super) fn tagged() -> GuardrailBase {
    let mut base = guardrail(Some("g"));
    base.settings.event_hook = Some(serde_json::from_str(r#"{"tags":{"a":"pre_call"}}"#).unwrap());
    base
}

pub(super) fn key(user: Option<&str>) -> StampedKeyFields {
    StampedKeyFields {
        user_id: user.map(str::to_owned),
        team_id: None,
        end_user_id: None,
        api_key: None,
        request_route: None,
    }
}

pub(super) async fn run_pre(
    base: &GuardrailBase,
    hooks: &Recorder,
    kwargs: Data,
    call_type: Option<CallType>,
) -> Result<Data, DeploymentHookError> {
    let marker_token = token();
    let suppressed = BTreeSet::new();
    let context = PreCallContext {
        token: &marker_token,
        suppressed: &suppressed,
    };
    base.pre_call_deployment_hook(hooks, &context, kwargs, call_type)
        .await
}

pub(super) const HOOKED: &str = r#"{"messages":[{"role":"user","content":"hooked"}]}"#;
pub(super) const GUARDED: &str =
    r#"{"guardrails":["g"],"messages":[{"role":"user","content":"hi"}]}"#;

pub(super) fn tag_mode_error() -> DeploymentHookError {
    DeploymentHookError::Selection(GuardrailSelectionError::TagMode {
        guardrail_name: Some("g".to_owned()),
    })
}

pub(super) const SCAN_REQUEST: &str = r#"{"model":"m","stream":true,"metadata":{"user_api_key_user_id":"u"},"messages":[{"role":"user","content":"old"}],"input":"x","prompt":"x","system":"x","instructions":"x","tools":[],"functions":[],"function_call":"auto","tool_choice":"auto"}"#;
pub(super) const GIVEN: &str = r#"[{"role":"tool","content":"retrieved"}]"#;

pub(super) fn given() -> Vec<GuardrailValue> {
    serde_json::from_str(GIVEN).unwrap()
}

pub(super) fn rejected(message: &str) -> MessagesHookError {
    MessagesHookError::Rejected {
        message: message.to_owned(),
        guardrail_name: Some("g".to_owned()),
        event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
    }
}

pub(super) fn pre_call_guardrail() -> GuardrailBase {
    with_hook(guardrail(Some("g")), "pre_call")
}

pub(super) fn reroute(name: Option<&str>) -> GuardrailFailure {
    GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
        route_to_model: "safe-model".to_owned(),
        session_id: "s".to_owned(),
        guardrail_name: name.map(str::to_owned),
        detection_info: Data::new(),
        message: None,
        sticky_session_routing: true,
    }))
}

pub(super) fn legacy_call<'call>(
    kwargs: Data,
    result: &'call GuardrailValue,
    call_type: &'call str,
) -> LoggedCall<'call> {
    LoggedCall {
        kwargs,
        result,
        result_kind: LoggedResultKind::Other,
        call_type,
    }
}

pub(super) fn shared() -> Arc<Mutex<Vec<Seen>>> {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn scanner_registry(seen: &Arc<Mutex<Vec<Seen>>>) -> TranslationRegistry<Scanner> {
    registry(&[(CallType::Acompletion, Scanner::new("acompletion", seen))])
}

pub(super) const LOGGED: &str = r#"{"model":"m","litellm_call_id":"c1","messages":[{"role":"user","content":"hi"}],"optional_params":{"tools":[{"type":"function"}]},"litellm_params":{"metadata":{"k":"v","standard_logging_guardrail_information":[{"old":1}]}}}"#;

pub(super) fn two_translations(seen: &Arc<Mutex<Vec<Seen>>>) -> TranslationRegistry<Scanner> {
    registry(&[
        (CallType::Completion, Scanner::new("completion", seen)),
        (CallType::Acompletion, Scanner::new("acompletion", seen)),
    ])
}

pub(super) const VERDICT: &str = r#"{"guardrail_name":"g","guardrail_status":"success"}"#;

pub(super) fn recording_registry(
    verdict: Option<&str>,
    fail_input: bool,
) -> TranslationRegistry<Scanner> {
    let seen = shared();
    let mut scanner = Scanner::new("acompletion", &seen);
    scanner.verdict = verdict.map(value);
    scanner.fail_input = fail_input;
    registry(&[(CallType::Acompletion, scanner)])
}

pub(super) async fn record(logged: &str, verdict: Option<&str>) -> Result<Data, LoggingHookError> {
    let result = GuardrailValue::Null;
    applying(Some("g"))
        .logging_hook(
            &(),
            &recording_registry(verdict, false),
            legacy_call(data(logged), &result, "acompletion"),
        )
        .await
}

pub(super) async fn pre_completion(
    hooks: &Recorder,
    kwargs: Data,
) -> Result<Data, DeploymentHookError> {
    run_pre(
        &guardrail(Some("g")),
        hooks,
        kwargs,
        Some(CallType::Completion),
    )
    .await
}

pub(super) async fn scan_logged(
    base: &GuardrailBase,
    logged: &str,
    result: &str,
    call_type: &str,
) -> (Data, Vec<Seen>) {
    let seen = shared();
    let result = value(result);
    let returned = base
        .logging_hook(
            &(),
            &scanner_registry(&seen),
            legacy_call(data(logged), &result, call_type),
        )
        .await
        .unwrap();
    let scans = seen.lock().unwrap().clone();
    (returned, scans)
}
