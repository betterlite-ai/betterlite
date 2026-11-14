#![cfg(test)]

use std::convert::Infallible;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::mem::take;
use std::ptr::from_ref;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::error::RealtimeGuardrailPolicyError;
use super::{
    RealtimeGuardrailCallback, TRANSCRIPTION_EVENT_HOOKS, disable_auto_response_message,
    guardrails_run_for_hooks, inject_auto_response_disable,
};
use crate::api::request_destination::RequestData;

struct CallAnswer {
    native_lifecycle: bool,
    event: GuardrailEventHooks,
    data_address: usize,
}

struct Scripted {
    custom: bool,
    native_lifecycle: bool,
    runs_for: Vec<GuardrailEventHooks>,
    answers: Mutex<Vec<CallAnswer>>,
}

impl RealtimeGuardrailCallback for Scripted {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        self.custom
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CallAnswer {
                native_lifecycle: self.native_lifecycle,
                event,
                data_address: from_ref(data).addr(),
            });
        Ok(self.runs_for.contains(&event))
    }
}

fn parsed(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn request_data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => take(&mut messages),
            Err(poisoned) => take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn captured<T, Call>(call: Call) -> (T, Vec<String>)
where
    Call: FnOnce() -> T,
{
    let logs = Captured::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}

#[rstest]
#[case::pre_call(vec![GuardrailEventHooks::PreCall], false)]
#[case::post_call(vec![GuardrailEventHooks::PostCall], false)]
#[case::during_call(vec![GuardrailEventHooks::DuringCall], false)]
#[case::transcription(vec![GuardrailEventHooks::RealtimeInputTranscription], true)]
#[case::pre_and_post(vec![GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall], false)]
#[case::every_hook(
    vec![
        GuardrailEventHooks::PreCall,
        GuardrailEventHooks::PostCall,
        GuardrailEventHooks::RealtimeInputTranscription,
    ],
    true
)]
fn transcription_predicate_counts_only_the_transcription_hook(
    #[case] runs_for: Vec<GuardrailEventHooks>,
    #[case] expected: bool,
) {
    let callback = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for,
        answers: Mutex::new(Vec::new()),
    };
    let detected = guardrails_run_for_hooks(
        &[&callback],
        &RequestData::default(),
        &TRANSCRIPTION_EVENT_HOOKS,
    );
    assert_eq!(detected.unwrap(), expected);
}

#[test]
fn non_guardrail_callbacks_never_answer_the_predicate() {
    let plain = Scripted {
        custom: false,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    assert!(
        !guardrails_run_for_hooks(
            &[&plain],
            &RequestData::default(),
            &TRANSCRIPTION_EVENT_HOOKS
        )
        .unwrap()
    );
    assert!(plain.answers.lock().unwrap().is_empty());
}

#[test]
fn native_lifecycle_guardrails_still_enable_detection() {
    let native = Scripted {
        custom: true,
        native_lifecycle: true,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    assert!(
        guardrails_run_for_hooks(
            &[&native],
            &RequestData::default(),
            &TRANSCRIPTION_EVENT_HOOKS
        )
        .unwrap()
    );
    assert_eq!(native.answers.lock().unwrap().len(), 1);
    assert!(native.answers.lock().unwrap()[0].native_lifecycle);
}

#[test]
fn a_false_predicate_disables_the_audio_gate() {
    let silent = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: Vec::new(),
        answers: Mutex::new(Vec::new()),
    };
    assert!(
        !guardrails_run_for_hooks(
            &[&silent],
            &RequestData::default(),
            &TRANSCRIPTION_EVENT_HOOKS
        )
        .unwrap()
    );
    assert_eq!(silent.answers.lock().unwrap().len(), 1);
}

#[test]
fn event_hooks_stop_at_the_first_match() {
    let first = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::RealtimeInputTranscription,
        ],
        answers: Mutex::new(Vec::new()),
    };
    let later = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::PreCall],
        answers: Mutex::new(Vec::new()),
    };
    let hooks = [
        GuardrailEventHooks::PreCall,
        GuardrailEventHooks::PostCall,
        GuardrailEventHooks::RealtimeInputTranscription,
    ];
    assert!(guardrails_run_for_hooks(&[&first, &later], &RequestData::default(), &hooks).unwrap());
    assert_eq!(first.answers.lock().unwrap().len(), 1);
    assert_eq!(
        first.answers.lock().unwrap()[0].event,
        GuardrailEventHooks::PreCall
    );
    assert!(later.answers.lock().unwrap().is_empty());
}

#[test]
fn no_event_hooks_means_no_guardrail_runs() {
    let callback = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::PreCall],
        answers: Mutex::new(Vec::new()),
    };
    assert!(!guardrails_run_for_hooks(&[&callback], &RequestData::default(), &[]).unwrap());
    assert!(callback.answers.lock().unwrap().is_empty());
}

#[test]
fn the_predicate_receives_the_request_data_unchanged() {
    let callback = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let data = request_data(r#"{"model": "gpt-4", "metadata": {"tags": ["release"]}}"#);
    assert!(guardrails_run_for_hooks(&[&callback], &data, &TRANSCRIPTION_EVENT_HOOKS).unwrap());
    assert_eq!(
        callback.answers.lock().unwrap()[0].data_address,
        from_ref(&data).addr()
    );
    assert_eq!(
        data,
        request_data(r#"{"model": "gpt-4", "metadata": {"tags": ["release"]}}"#)
    );
}

#[test]
fn beta_disable_message_disables_vad_auto_response() {
    let message = disable_auto_response_message(true).unwrap();
    assert_eq!(
        parsed(&message),
        parsed(
            r#"{"type": "session.update", "session": {"turn_detection": {"type": "server_vad", "create_response": false}}}"#
        )
    );
}

#[test]
fn ga_disable_message_nests_turn_detection_under_audio_input() {
    let message = disable_auto_response_message(false).unwrap();
    assert_eq!(
        parsed(&message),
        parsed(
            r#"{"type": "session.update", "session": {"type": "realtime", "audio": {"input": {"turn_detection": {"type": "server_vad", "create_response": false}}}}}"#
        )
    );
}

#[test]
fn setup_injection_creates_missing_nested_objects() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {"model": "gpt-4"}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    )
    .unwrap();
    assert!(sent);
    let expected = parsed(
        r#"{"setup": {"model": "gpt-4", "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}}}}"#,
    );
    assert_eq!(parsed(&injected.unwrap()), expected);
}

#[test]
fn setup_injection_keeps_existing_configuration_fields() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {"realtimeInputConfig": {"automaticActivityDetection": {"sensitivity": "high"}, "prefixPaddingMs": 20}}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    )
    .unwrap();
    assert!(sent);
    let expected = parsed(
        r#"{"setup": {"realtimeInputConfig": {"automaticActivityDetection": {"sensitivity": "high", "disabled": true}, "prefixPaddingMs": 20}}}"#,
    );
    assert_eq!(parsed(&injected.unwrap()), expected);
}

#[test]
fn setup_injection_overwrites_an_existing_disabled_flag() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {"realtimeInputConfig": {"automaticActivityDetection": {"disabled": false}}}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    )
    .unwrap();
    assert!(sent);
    let expected = parsed(
        r#"{"setup": {"realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}}}}"#,
    );
    assert_eq!(parsed(&injected.unwrap()), expected);
}

#[test]
fn setup_injection_logs_the_fold_diagnostic() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let (injected, logs) = captured(|| {
        inject_auto_response_disable(
            r#"{"setup": {}}"#,
            &mut sent,
            &[&guardrail],
            &RequestData::default(),
        )
    });
    assert!(injected.unwrap().is_some());
    assert_eq!(
        logs,
        vec![
            "Realtime: folded automaticActivityDetection.disabled=true into setup for transcription-guardrail gating"
                .to_owned()
        ]
    );
}

#[test]
fn setup_injection_leaves_an_already_sent_setup_untouched() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = true;
    let injected = inject_auto_response_disable(
        r#"{"setup": {}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    )
    .unwrap();
    assert!(injected.is_none());
    assert!(sent);
    assert!(guardrail.answers.lock().unwrap().is_empty());
}

#[test]
fn setup_injection_without_transcription_guardrails_is_untouched() {
    let chat_only = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {}}"#,
        &mut sent,
        &[&chat_only],
        &RequestData::default(),
    )
    .unwrap();
    assert!(injected.is_none());
    assert!(!sent);
}

#[rstest]
#[case::invalid_json("not json")]
#[case::json_scalar(r#""5""#)]
#[case::json_list("[]")]
#[case::missing_setup(r#"{"type": "session.update", "session": {"turn_detection": {}}}"#)]
#[case::null_setup(r#"{"setup": null}"#)]
#[case::list_setup(r#"{"setup": []}"#)]
#[case::text_setup(r#"{"setup": "realtime"}"#)]
fn setup_injection_ignores_messages_without_a_setup_object(#[case] message: &str) {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected =
        inject_auto_response_disable(message, &mut sent, &[&guardrail], &RequestData::default())
            .unwrap();
    assert!(injected.is_none());
    assert!(!sent);
}

#[test]
fn setup_injection_rejects_a_nonobject_realtime_input_config() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {"realtimeInputConfig": "none"}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    );
    assert!(matches!(
        injected,
        Err(RealtimeGuardrailPolicyError::SetupFieldNotObject {
            field: "realtimeInputConfig"
        })
    ));
    assert!(!sent);
}

#[test]
fn setup_injection_rejects_a_nonobject_automatic_activity_detection() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {"realtimeInputConfig": {"automaticActivityDetection": null}}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    );
    assert!(matches!(
        injected,
        Err(RealtimeGuardrailPolicyError::SetupFieldNotObject {
            field: "automaticActivityDetection"
        })
    ));
    assert!(!sent);
}

#[test]
fn setup_injection_marks_the_session_before_the_caller_sends() {
    let guardrail = Scripted {
        custom: true,
        native_lifecycle: false,
        runs_for: vec![GuardrailEventHooks::RealtimeInputTranscription],
        answers: Mutex::new(Vec::new()),
    };
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {}}"#,
        &mut sent,
        &[&guardrail],
        &RequestData::default(),
    )
    .unwrap();
    assert!(injected.is_some());
    assert!(sent);
    let delivery: Result<(), &str> = Err("the backend closed before the setup arrived");
    assert!(delivery.is_err());
    assert!(sent);
}

#[derive(Debug, PartialEq, Eq)]
struct GateFailure;

impl Display for GateFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("the guardrail gate failed")
    }
}

impl Error for GateFailure {}

struct Failing;

impl RealtimeGuardrailCallback for Failing {
    type Error = GateFailure;

    fn is_custom_guardrail(&self) -> bool {
        true
    }

    fn should_run_guardrail(
        &self,
        _data: &RequestData,
        _event: GuardrailEventHooks,
    ) -> Result<bool, GateFailure> {
        Err(GateFailure)
    }
}

#[test]
fn a_failing_predicate_error_is_returned_not_read_as_false() {
    let detected = guardrails_run_for_hooks(
        &[&Failing],
        &RequestData::default(),
        &TRANSCRIPTION_EVENT_HOOKS,
    );
    assert_eq!(detected, Err(GateFailure));
}

#[test]
fn setup_injection_surfaces_a_failing_predicate_and_leaves_the_flag_unset() {
    let mut sent = false;
    let injected = inject_auto_response_disable(
        r#"{"setup": {}}"#,
        &mut sent,
        &[&Failing],
        &RequestData::default(),
    );
    assert!(matches!(
        injected,
        Err(RealtimeGuardrailPolicyError::Callback(GateFailure))
    ));
    assert!(!sent);
}
