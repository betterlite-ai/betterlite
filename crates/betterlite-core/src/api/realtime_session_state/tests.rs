#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tracing::field::Field;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::logging::{RealtimeLogging, RealtimeSuccessQueue, SessionInput, SuccessWork};
use super::store::SessionMessage;
use super::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming, ResponseAssembly, SessionFlag,
    SessionSetup,
};
use crate::api::realtime_contracts::events::{AudioTranscriptionUsage, DurationUsage};

fn record(source: &str) -> IndexMap<String, Json> {
    let value: Json = serde_json::from_str(source).expect("the test record is valid JSON");
    match value {
        Json::Object(record) => record,
        other => panic!("the test record is an object, not {other:?}"),
    }
}

fn duration_usage(seconds: u64) -> AudioTranscriptionUsage {
    AudioTranscriptionUsage::Duration(DurationUsage {
        seconds: serde_json::Number::from(seconds),
    })
}

fn connection() -> ConnectionSetup {
    ConnectionSetup {
        client: None,
        backend: None,
        logging: None,
        logging_worker: None,
        provider_config: None,
        event_normalizer: None,
        logged_real_time_event_types: LoggedEventTypes::Default,
        model: String::new(),
        user_api_key_dict: None,
        request_data: None,
        force_transcription_model: None,
        client_wants_beta: false,
        backend_uses_beta_protocol: None,
    }
}

fn state() -> RealtimeStreaming {
    RealtimeStreaming::new(connection())
}

fn state_with<Configure>(configure: Configure) -> RealtimeStreaming
where
    Configure: FnOnce(&mut ConnectionSetup),
{
    let mut connection = connection();
    configure(&mut connection);
    RealtimeStreaming::new(connection)
}

fn provider(
    requires: bool,
    usage: Option<AudioTranscriptionUsage>,
) -> Box<dyn RealtimeProviderConfig> {
    Box::new(Provider { requires, usage })
}

struct Provider {
    requires: bool,
    usage: Option<AudioTranscriptionUsage>,
}

impl RealtimeProviderConfig for Provider {
    fn requires_session_configuration(&self) -> bool {
        self.requires
    }

    fn unbilled_usage_on_close(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        self.usage.clone()
    }
}

#[derive(Clone, Default)]
struct Spy {
    inner: Arc<Mutex<SpyState>>,
}

#[derive(Default)]
struct SpyState {
    accepted: bool,
    pre_calls: Vec<(Json, String)>,
    details: Vec<(String, Json)>,
    dispatches: Vec<Vec<IndexMap<String, Json>>>,
    enqueues: usize,
}

impl Spy {
    fn lock(&self) -> MutexGuard<'_, SpyState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn accept(&self) {
        self.lock().accepted = true;
    }

    fn details(&self) -> Vec<(String, Json)> {
        self.lock().details.clone()
    }

    fn flag(&self) -> Option<Json> {
        self.lock()
            .details
            .iter()
            .find(|(key, _)| key == super::logging::SUCCESS_LOGGED_KEY)
            .map(|(_, value)| value.clone())
    }
}

impl RealtimeLogging for Spy {
    fn pre_call(&self, input: &SessionInput<'_>, api_key: &str) {
        let value = match input {
            SessionInput::Text(text) => Json::Text((*text).to_owned()),
            SessionInput::Record(record) => Json::Object((*record).clone()),
        };
        self.lock().pre_calls.push((value, api_key.to_owned()));
    }

    fn record_model_call_detail(&self, key: &str, value: Json) {
        self.lock().details.push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, messages: &[IndexMap<String, Json>]) -> SuccessWork {
        self.lock().dispatches.push(messages.to_vec());
        Box::pin(async {})
    }
}

impl RealtimeSuccessQueue for Spy {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        let mut state = self.lock();
        state.enqueues += 1;
        state.accepted
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

impl Capture {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, message)| message.clone())
            .collect()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((*event.metadata().level(), visitor.0));
        }
    }
}

fn capture() -> (Capture, DefaultGuard) {
    let logs = Capture::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

fn store_types(state: &mut RealtimeStreaming, types: &[&str]) {
    for event_type in types {
        let frame = format!("{{\"type\":\"{event_type}\"}}");
        state
            .store_message(SessionMessage::Text(&frame))
            .expect("a typed event frame is valid JSON");
    }
}

#[test]
fn realtime_initial_state() {
    let state = state();
    assert!(state.messages.is_empty());
    assert!(state.input_message.is_empty());
    assert!(state.input_messages.is_empty());
    assert!(state.session_tools.is_empty());
    assert!(state.tool_calls.is_empty());
    assert_eq!(state.model, "");
    assert!(state.request_data.is_empty());
    assert_eq!(
        state.logged_real_time_event_types,
        LoggedEventTypes::Default
    );
    assert_eq!(state.response, ResponseAssembly::default());
    assert_eq!(state.setup, SessionSetup::new(true));
    assert_eq!(state.force_transcription_model, None);
    assert_eq!(state.is_transcription_session, SessionFlag(false));
    assert_eq!(state.client_wants_beta, SessionFlag(false));
    assert_eq!(state.backend_uses_beta_protocol, SessionFlag(false));
    assert_eq!(state.backend_sent_frames, SessionFlag(false));
}

#[test]
fn realtime_initial_state_follows_provider_setup_requirements() {
    let requiring = state_with(|connection| {
        connection.provider_config = Some(provider(true, None));
    });
    let relaxed = state_with(|connection| {
        connection.provider_config = Some(provider(false, None));
    });
    assert_eq!(requiring.setup.backend_setup_complete, SessionFlag(true));
    assert_eq!(relaxed.setup.backend_setup_complete, SessionFlag(false));
}

#[test]
fn realtime_initial_state_keeps_beta_preferences_separate() {
    let explicit = state_with(|connection| {
        connection.client_wants_beta = true;
        connection.backend_uses_beta_protocol = Some(false);
    });
    assert_eq!(explicit.client_wants_beta, SessionFlag(true));
    assert_eq!(explicit.backend_uses_beta_protocol, SessionFlag(false));
    let inherited = state_with(|connection| {
        connection.client_wants_beta = true;
    });
    assert_eq!(inherited.backend_uses_beta_protocol, SessionFlag(true));
}

#[test]
fn realtime_initial_state_marks_a_forced_transcription_session() {
    let forced = state_with(|connection| {
        connection.force_transcription_model = Some(String::new());
    });
    assert_eq!(forced.force_transcription_model.as_deref(), Some(""));
    assert_eq!(forced.is_transcription_session, SessionFlag(true));
    assert_eq!(state().is_transcription_session, SessionFlag(false));
}

#[test]
fn realtime_initial_state_is_not_shared_between_connections() {
    let mut first = state();
    let mut second = state();
    first
        .store_message(SessionMessage::Text(r#"{"type":"response.create"}"#))
        .expect("the frame is valid JSON");
    first
        .input_messages
        .push(record(r#"{"role":"user","content":"one"}"#));
    first.setup.violation_count = 2;
    first
        .setup
        .pending_messages_until_setup
        .push("{}".to_owned());
    assert!(second.messages.is_empty());
    assert!(second.input_messages.is_empty());
    assert_eq!(second.setup.violation_count, 0);
    assert!(second.setup.pending_messages_until_setup.is_empty());
}

#[test]
fn realtime_event_storage_stores_the_five_default_types() {
    let mut state = state();
    store_types(
        &mut state,
        &[
            "session.created",
            "response.create",
            "response.done",
            "conversation.item.added",
            "conversation.item.done",
        ],
    );
    assert_eq!(state.messages.len(), 5);
    store_types(
        &mut state,
        &["response.output_text.delta", "conversation.item.deleted"],
    );
    state
        .store_message(SessionMessage::Text(r#"{"untyped":true}"#))
        .expect("an untyped frame is still valid JSON");
    assert_eq!(state.messages.len(), 5);
}

#[test]
fn realtime_event_storage_handles_wildcard_and_empty_selection() {
    let mut all = state_with(|connection| {
        connection.logged_real_time_event_types = LoggedEventTypes::All;
    });
    all.store_message(SessionMessage::Text(r#"{"untyped":true}"#))
        .expect("the frame is valid JSON");
    assert_eq!(all.messages.len(), 1);
    let mut none = state_with(|connection| {
        connection.logged_real_time_event_types = LoggedEventTypes::Only(Vec::new());
    });
    store_types(&mut none, &["response.done"]);
    assert!(none.messages.is_empty());
    let mut selected = state_with(|connection| {
        connection.logged_real_time_event_types =
            LoggedEventTypes::Only(vec!["response.done".to_owned()]);
    });
    store_types(&mut selected, &["response.create", "response.done"]);
    assert_eq!(
        selected.messages,
        vec![record(r#"{"type":"response.done"}"#)]
    );
}

#[test]
fn realtime_event_storage_accepts_bytes_strings_and_records() {
    let mut state = state();
    state
        .store_message(SessionMessage::Binary(br#"{"type":"response.create"}"#))
        .expect("the bytes hold a JSON object");
    state
        .store_message(SessionMessage::Text(r#"{"type":"response.done"}"#))
        .expect("the string holds a JSON object");
    let direct = record(r#"{"type":"session.created","session":{}}"#);
    state
        .store_message(SessionMessage::Record(&direct))
        .expect("a record is already decoded");
    assert_eq!(state.messages.len(), 3);
    assert_eq!(state.messages[2], direct);
}

#[test]
fn realtime_event_storage_fails_before_the_record_fallback() {
    let mut state = state();
    assert!(
        state
            .store_message(SessionMessage::Binary(&[0xff]))
            .is_err()
    );
    assert!(state.store_message(SessionMessage::Text("{")).is_err());
    assert!(state.store_message(SessionMessage::Text("[1]")).is_err());
    assert!(state.messages.is_empty());
}

#[test]
fn realtime_event_storage_logs_a_session_event_shape_failure() {
    let (logs, _guard) = capture();
    let mut state = state();
    state
        .store_message(SessionMessage::Text(
            r#"{"type":"session.created","session":{}}"#,
        ))
        .expect("a session event with an object session is stored");
    state
        .store_message(SessionMessage::Text(r#"{"type":"session.created"}"#))
        .expect("a session event without a session object is still stored");
    state
        .store_message(SessionMessage::Text(r#"{"type":"response.create"}"#))
        .expect("an ordinary event needs no session object");
    assert_eq!(state.messages.len(), 3);
    let failures: Vec<String> = logs
        .messages()
        .into_iter()
        .filter(|message| message.contains("Error parsing message for logging:"))
        .collect();
    assert_eq!(failures.len(), 1, "{failures:?}");
}

#[test]
fn realtime_event_storage_collects_response_tool_calls_before_filtering() {
    let mut state = state_with(|connection| {
        connection.logged_real_time_event_types = LoggedEventTypes::Only(Vec::new());
    });
    state
        .store_message(SessionMessage::Text(
            r#"{"type":"response.done","response":{"output":[{"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"city\":\"Paris\"}"},{"type":"message"},{"type":"function_call"}]}}"#,
        ))
        .expect("the response.done frame is valid JSON");
    assert!(state.messages.is_empty());
    assert_eq!(state.tool_calls.len(), 2);
    assert_eq!(
        state.tool_calls[0],
        record(
            r#"{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}}"#
        )
    );
    assert_eq!(
        state.tool_calls[1],
        record(r#"{"id":"","type":"function","function":{"name":"","arguments":"{}"}}"#)
    );
}

#[test]
fn realtime_event_storage_keeps_explicit_tool_call_values() {
    let mut state = state();
    state
        .store_message(SessionMessage::Text(
            r#"{"type":"response.done","response":{"output":[{"type":"function_call","call_id":null,"name":"","arguments":null}]}}"#,
        ))
        .expect("the response.done frame is valid JSON");
    assert_eq!(
        state.tool_calls,
        vec![record(
            r#"{"id":null,"type":"function","function":{"name":"","arguments":null}}"#
        )]
    );
}

#[test]
fn realtime_input_collection_keeps_a_record_and_clears_text_input() {
    let spy = Spy::default();
    let mut state = state_with(|connection| {
        connection.logging = Some(Box::new(spy.clone()));
    });
    let update = record(
        r#"{"type":"conversation.item.create","item":{"role":"user","content":[{"type":"input_text","text":"hello"},{"type":"input_audio","audio":"..."},{"type":"input_text","text":""},{"type":"input_text"}]}}"#,
    );
    state.store_input(SessionInput::Record(&update));
    assert_eq!(state.input_message, update);
    assert_eq!(
        state.input_messages,
        vec![record(r#"{"role":"user","content":"hello"}"#)]
    );
    state.store_input(SessionInput::Text("{}"));
    assert!(state.input_message.is_empty());
    let calls = spy.lock().pre_calls.clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, "");
    assert_eq!(calls[0].0, Json::Object(update));
    assert_eq!(calls[1].0, Json::Text("{}".to_owned()));
}

#[test]
fn realtime_input_collection_reads_session_updates() {
    let mut state = state();
    state.store_input(SessionInput::Text(
        r#"{"type":"session.update","session":{"instructions":"be brief","tools":[{"type":"function","name":"lookup"}],"type":"transcription"}}"#,
    ));
    assert_eq!(
        state.input_messages,
        vec![record(r#"{"role":"system","content":"be brief"}"#)]
    );
    assert_eq!(
        state.session_tools,
        vec![record(r#"{"type":"function","name":"lookup"}"#)]
    );
    assert_eq!(state.is_transcription_session, SessionFlag(true));
    state.store_input(SessionInput::Text(
        r#"{"type":"session.update","session":{"tools":[],"instructions":""}}"#,
    ));
    assert_eq!(state.session_tools.len(), 1);
    assert_eq!(state.input_messages.len(), 1);
    assert_eq!(state.is_transcription_session, SessionFlag(true));
    state.store_input(SessionInput::Text(
        r#"{"type":"session.update","session":{"tools":["not-a-tool",{"type":"function","name":"other"}]}}"#,
    ));
    assert_eq!(
        state.session_tools,
        vec![record(r#"{"type":"function","name":"lookup"}"#)]
    );
}

#[test]
fn realtime_input_collection_ignores_malformed_client_events() {
    let mut state = state();
    state.store_input(SessionInput::Text("{"));
    state.store_input(SessionInput::Text("[]"));
    state.store_input(SessionInput::Text(
        r#"{"type":"conversation.item.create","item":{"role":"user","content":"hello"}}"#,
    ));
    state.store_input(SessionInput::Text(
        r#"{"type":"conversation.item.create","item":"hello"}"#,
    ));
    state.store_input(SessionInput::Text(
        r#"{"type":"session.update","session":"hello"}"#,
    ));
    assert!(state.input_messages.is_empty());
    assert!(state.session_tools.is_empty());
    assert_eq!(state.is_transcription_session, SessionFlag(false));
}

#[test]
fn realtime_input_collection_takes_backend_transcripts() {
    let mut state = state();
    let event = record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello"}"#,
    );
    state.collect_backend_input(&event);
    state.collect_backend_input(&record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":""}"#,
    ));
    state.collect_backend_input(&record(
        r#"{"type":"response.done","transcript":"ignored"}"#,
    ));
    assert_eq!(
        state.input_messages,
        vec![record(r#"{"role":"user","content":"hello"}"#)]
    );
}

#[test]
fn realtime_input_collection_marks_transcription_sessions_from_backend_events() {
    let mut state = state();
    state.detect_transcription_session(&record(
        r#"{"type":"session.created","session":{"type":"transcription"}}"#,
    ));
    assert_eq!(state.is_transcription_session, SessionFlag(true));
    state.detect_transcription_session(&record(r#"{"type":"session.updated"}"#));
    assert_eq!(state.is_transcription_session, SessionFlag(true));

    let mut created = state();
    created.detect_transcription_session(&record(r#"{"type":"transcription_session.created"}"#));
    assert_eq!(created.is_transcription_session, SessionFlag(true));

    let mut updated = state();
    updated.detect_transcription_session(&record(r#"{"type":"transcription_session.updated"}"#));
    assert_eq!(updated.is_transcription_session, SessionFlag(true));

    let mut other = state();
    other.detect_transcription_session(&record(
        r#"{"type":"session.created","session":{"type":"realtime"}}"#,
    ));
    other.detect_transcription_session(&record(r#"{"type":"response.created"}"#));
    assert_eq!(other.is_transcription_session, SessionFlag(false));
}

#[test]
fn realtime_usage_and_logging_captures_usage_without_the_transcript() {
    let mut state = state();
    let event = record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello","usage":{"type":"duration","seconds":2}}"#,
    );
    state.collect_backend_input(&event);
    state.capture_transcription_usage(&event);
    assert_eq!(
        state.messages,
        vec![record(
            r#"{"type":"conversation.item.input_audio_transcription.completed","usage":{"type":"duration","seconds":2}}"#
        )]
    );
    assert_eq!(
        state.input_messages,
        vec![record(r#"{"role":"user","content":"hello"}"#)]
    );
}

#[test]
fn realtime_usage_and_logging_does_not_duplicate_a_logged_event() {
    let mut state = state_with(|connection| {
        connection.logged_real_time_event_types = LoggedEventTypes::All;
    });
    let event = record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello","usage":{"type":"duration","seconds":2}}"#,
    );
    state
        .store_message(SessionMessage::Record(&event))
        .expect("the record is already decoded");
    state.capture_transcription_usage(&event);
    assert_eq!(state.messages, vec![event]);
}

#[test]
fn realtime_usage_and_logging_ignores_events_without_usage() {
    let mut state = state();
    state.capture_transcription_usage(&record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello"}"#,
    ));
    state.capture_transcription_usage(&record(
        r#"{"type":"conversation.item.input_audio_transcription.completed","usage":null}"#,
    ));
    assert!(state.messages.is_empty());
}

#[test]
fn realtime_usage_and_logging_flushes_provider_usage_on_close() {
    let mut state = state_with(|connection| {
        connection.provider_config = Some(provider(true, Some(duration_usage(2))));
    });
    state
        .flush_unbilled_usage()
        .expect("the usage record encodes");
    assert_eq!(
        state.messages,
        vec![record(
            r#"{"type":"conversation.item.input_audio_transcription.completed","usage":{"type":"duration","seconds":2}}"#
        )]
    );

    let mut empty = state_with(|connection| {
        connection.provider_config = Some(provider(true, None));
    });
    empty.flush_unbilled_usage().expect("nothing is flushed");
    assert!(empty.messages.is_empty());

    let mut bare = state();
    bare.flush_unbilled_usage().expect("nothing is flushed");
    assert!(bare.messages.is_empty());
}

#[test]
fn realtime_usage_and_logging_copies_details_and_queues_once() {
    let spy = Spy::default();
    spy.accept();
    let mut state = state_with(|connection| {
        connection.logging = Some(Box::new(spy.clone()));
        connection.logging_worker = Some(Box::new(spy.clone()));
    });
    state
        .input_messages
        .push(record(r#"{"role":"user","content":"hello"}"#));
    state
        .session_tools
        .push(record(r#"{"type":"function","name":"lookup"}"#));
    state.tool_calls.push(record(
        r#"{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}"#,
    ));
    assert!(state.log_messages());
    let details = spy.details();
    assert_eq!(details[0].0, "messages");
    assert_eq!(details[1].0, "realtime_tools");
    assert_eq!(details[2].0, "realtime_tool_calls");
    assert_eq!(spy.flag(), Some(Json::Bool(true)));
    assert_eq!(spy.lock().dispatches.len(), 1);
    assert_eq!(spy.lock().dispatches[0], state.messages);
    assert_eq!(spy.lock().enqueues, 1);
}

#[test]
fn realtime_usage_and_logging_writes_both_tool_keys_only_when_one_is_set() {
    let spy = Spy::default();
    spy.accept();
    let mut state = state_with(|connection| {
        connection.logging = Some(Box::new(spy.clone()));
        connection.logging_worker = Some(Box::new(spy.clone()));
    });
    state.tool_calls.push(record(
        r#"{"id":"","type":"function","function":{"name":"","arguments":"{}"}}"#,
    ));
    assert!(state.log_messages());
    let keys: Vec<String> = spy.details().into_iter().map(|(key, _)| key).collect();
    assert_eq!(
        keys,
        vec![
            "realtime_tools",
            "realtime_tool_calls",
            "realtime_session_success_logged"
        ]
    );
    assert_eq!(spy.details()[0].1, Json::List(Vec::new()));
}

#[test]
fn realtime_usage_and_logging_leaves_the_flag_unset_without_a_queue_admission() {
    let spy = Spy::default();
    let mut state = state_with(|connection| {
        connection.logging = Some(Box::new(spy.clone()));
        connection.logging_worker = Some(Box::new(spy.clone()));
    });
    state
        .input_messages
        .push(record(r#"{"role":"user","content":"hello"}"#));
    assert!(!state.log_messages());
    assert_eq!(spy.flag(), None);
    assert_eq!(spy.details()[0].0, "messages");
    assert_eq!(spy.lock().enqueues, 1);
}

#[test]
fn realtime_usage_and_logging_ignores_a_connection_without_a_logger() {
    let spy = Spy::default();
    spy.accept();
    let mut state = state_with(|connection| {
        connection.logging_worker = Some(Box::new(spy.clone()));
    });
    state
        .input_messages
        .push(record(r#"{"role":"user","content":"hello"}"#));
    assert!(!state.log_messages());
    assert_eq!(spy.lock().enqueues, 0);
    assert!(spy.details().is_empty());
}
