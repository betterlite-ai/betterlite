use std::fmt::Debug;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;

use crate::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{
    ApiCallLogger, InputCallback, InputEventSink, PostCallEvent, PreCallEvent, SentrySink,
};
use crate::logging_pre_post::post_call::post_call;
use crate::logging_pre_post::pre_call::{PreCallOutput, pre_call};
use crate::logging_privacy::request::Decision;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::python_json;

const BASE_INIT: &str = r#""model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1""#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Knob {
    Verbose,
    JsonLogs,
    RequestDebug,
    RedactContent,
    GlobalRawLogging,
    TurnOffMessages,
    Sentry,
    FailingLoggerFn,
    FailingCallback,
    EarlierAttempt,
    NoLoggerFn,
}

pub(super) struct Recorder {
    pub events: Mutex<Vec<String>>,
    pub records: Mutex<Vec<LogRecord>>,
    pub failing: bool,
}

impl Recorder {
    fn new(failing: bool) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            records: Mutex::new(Vec::new()),
            failing,
        }
    }

    fn push(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    pub(super) fn records(&self) -> Vec<LogRecord> {
        self.records.lock().unwrap().clone()
    }

    fn finish(&self, entry: String) -> Result<(), PrePostError> {
        self.push(entry);
        if self.failing {
            return Err(PrePostError::CallbackFailed {
                message: "callback exploded".to_owned(),
            });
        }
        Ok(())
    }
}

impl LogSink for Recorder {
    fn emit(&self, record: LogRecord) {
        self.push(format!("log:{:?}:{}", record.level, record.message));
        self.records.lock().unwrap().push(record);
    }
}

impl ApiCallLogger for Recorder {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        let messages = python_json::dumps(event.messages);
        let details = python_json::dumps(event.details);
        self.finish(format!("logger_pre:{}:{messages}:{details}", event.model))
    }

    fn log_post_api_call(&self, event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        let start = event.start_time.to_rfc3339();
        self.finish(format!(
            "logger_post:{start}:{}",
            python_json::dumps(event.details)
        ))
    }
}

impl InputEventSink for Recorder {
    fn log_input_event(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        self.push(format!("input_event:{}", event.model));
        Ok(())
    }
}

impl SentrySink for Recorder {
    fn add_breadcrumb(&self, message: &str) -> Result<(), PrePostError> {
        self.push(format!("sentry:{message}"));
        Ok(())
    }

    fn capture_exception(&self, message: &str) {
        self.push(format!("captured:{message}"));
    }
}

pub(super) struct Run<T> {
    pub state: LoggingState,
    pub result: T,
    pub recorder: Recorder,
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn detail<'state>(state: &'state LoggingState, path: &[&str]) -> Option<&'state Json> {
    let (first, rest) = path.split_first()?;
    let mut current = state.model_call_details.get(*first)?;
    for name in rest {
        let Json::Object(fields) = current else {
            return None;
        };
        current = fields.get(*name)?;
    }
    Some(current)
}

pub(super) fn state(extra: &str) -> LoggingState {
    let init: LoggingInit =
        serde_json::from_str(&format!("{{{BASE_INIT}{extra}}}")).expect("the fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}

fn settings(knobs: &[Knob]) -> (DebugSettings, GlobalFlags, Decision) {
    (
        DebugSettings {
            verbose: knobs.contains(&Knob::Verbose),
            json_logs: knobs.contains(&Knob::JsonLogs),
        },
        GlobalFlags {
            log_raw_request_response: knobs.contains(&Knob::GlobalRawLogging),
            turn_off_message_logging: knobs.contains(&Knob::TurnOffMessages),
        },
        Decision {
            content: knobs.contains(&Knob::RedactContent),
            key_info: false,
        },
    )
}

pub(super) fn run<T, F>(extra: &str, knobs: &[Knob], action: F) -> Run<T>
where
    F: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let mut logging = state(extra);
    logging.litellm_request_debug = knobs.contains(&Knob::RequestDebug);
    let recorder = Recorder::new(knobs.contains(&Knob::FailingCallback));
    let scrubber = redactor();
    let callbacks = [
        InputCallback::Sentry,
        InputCallback::Logger(&recorder),
        InputCallback::InputEvent(&recorder),
    ];
    let logger_fn = |details: &Json| {
        recorder.push(format!("logger_fn:{}", python_json::dumps(details)));
        if knobs.contains(&Knob::FailingLoggerFn) {
            return Err(PrePostError::CallbackFailed {
                message: "logger_fn exploded".to_owned(),
            });
        }
        Ok(())
    };
    let sentry: &dyn SentrySink = &recorder;
    let logger_hook: LoggerFn<'_> = &logger_fn;
    let (debug, globals, decision) = settings(knobs);
    let env = PrePostEnv {
        debug,
        globals,
        decision,
        redactor: &scrubber,
        log: &recorder,
        input_callbacks: &callbacks,
        sentry: knobs.contains(&Knob::Sentry).then_some(sentry),
        logger_fn: (!knobs.contains(&Knob::NoLoggerFn)).then_some(logger_hook),
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the fixture parses"),
    };
    let result = action(&mut logging, &env);
    Run {
        state: logging,
        result,
        recorder,
    }
}

pub(super) type PreRun = Run<Result<PreCallOutput, PrePostError>>;

pub(super) const CALL: &str = r#"{"input":[{"role":"user","content":"call input"}],"additional_args":{"api_base":"https://host/v1?key=abcdef1234","headers":{"Authorization":"Bearer sk-live-abcdef123456","x-trace":"t1"},"complete_input_dict":{"model":"gpt-4o"}}}"#;

pub(super) fn run_pre(extra: &str, call: &str, knobs: &[Knob]) -> PreRun {
    run(extra, knobs, |logging, env| {
        let earlier = Knob::EarlierAttempt;
        if knobs.contains(&earlier) {
            let earlier_start = text("earlier");
            logging
                .model_call_details
                .insert("first_api_call_start_time".to_owned(), earlier_start);
        }
        pre_call(
            logging,
            &serde_json::from_str(call).expect("the call fixture is valid"),
            env,
        )
    })
}

pub(super) fn run_post(extra: &str, call: &str, knobs: &[Knob]) -> Run<()> {
    run(extra, knobs, |logging, env| {
        let parsed = serde_json::from_str(call).expect("the call fixture is valid");
        post_call(logging, &parsed, env);
    })
}

pub(super) enum Look<'look> {
    Detail(&'look [&'look str]),
    Param(&'look str),
    Kinds,
    Record(usize),
    Outcome,
    Leaks(&'look str),
    Mentions(&'look str, &'look str),
}

fn kind(event: &str) -> String {
    let mut parts = event.split(':');
    let first = parts.next().unwrap_or_default();
    if first == "log" {
        return format!("log:{}", parts.next().unwrap_or_default());
    }
    first.to_owned()
}

fn record_text(recorder: &Recorder, index: usize) -> String {
    let records = recorder.records();
    let Some(record) = records.get(index) else {
        return "none".to_owned();
    };
    format!(
        "{:?}:{}:{}",
        record.level,
        record.message,
        python_json::dumps(&Json::Object(record.fields.clone()))
    )
}

fn mentions(recorder: &Recorder, prefix: &str, needle: &str) -> bool {
    recorder
        .events()
        .iter()
        .any(|event| event.starts_with(prefix) && event.contains(needle))
}

pub(super) fn look<T>(run: &Run<T>, what: &Look<'_>) -> String
where
    T: Debug,
{
    let all = python_json::dumps(&Json::Object(run.state.model_call_details.clone()));
    match what {
        Look::Detail(path) => {
            detail(&run.state, path).map_or_else(|| "absent".to_owned(), python_json::dumps)
        }
        Look::Param(name) => run
            .state
            .litellm_params
            .get(*name)
            .map_or_else(|| "absent".to_owned(), python_json::dumps),
        Look::Kinds => run
            .recorder
            .events()
            .iter()
            .map(|event| kind(event))
            .collect::<Vec<_>>()
            .join("|"),
        Look::Record(index) => record_text(&run.recorder, *index),
        Look::Outcome => format!("{:?}", run.result),
        Look::Leaks(needle) if all.contains(needle) => "leaked".to_owned(),
        Look::Leaks(_) => "clean".to_owned(),
        Look::Mentions(prefix, needle) if mentions(&run.recorder, prefix, needle) => {
            "yes".to_owned()
        }
        Look::Mentions(_, _) => "no".to_owned(),
    }
}
