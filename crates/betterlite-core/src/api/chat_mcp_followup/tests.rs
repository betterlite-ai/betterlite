#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::message_utilities::validators::ValidationError;

use super::ChatFollowupError;
use super::McpToolResult;
use super::create_follow_up_messages_for_chat;

const CONVERSION_FAILED: &str = "Failed to convert assistant message for MCP flow";
const TOOL_CALLS_RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"no","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":""}},{"id":"call_2","type":"function","function":{"name":"forecast","arguments":"{\"city\":\"Oslo\"}"}}]},"finish_reason":"tool_calls"},{"index":1,"message":{"role":"assistant","content":"a later choice"},"finish_reason":"stop"}]}"#;
const EMPTY_RESPONSE: &str =
    r#"{"id":"chatcmpl-2","object":"chat.completion","created":2,"model":"gpt-4o","choices":[]}"#;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Logged(String);

impl Visit for Logged {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut logged = Logged::default();
        event.record(&mut logged);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(logged.0);
    }
}

fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    (set_default(subscriber), captured)
}

fn lines(captured: &Captured) -> Vec<String> {
    captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a valid JSON fixture")
}

fn messages(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("a valid message list fixture")
}

fn response(source: &str) -> ChatResponse {
    serde_json::from_str(source).expect("a valid chat response fixture")
}

fn response_with_message(message: &str) -> ChatResponse {
    response(&format!(
        r#"{{"id":"chatcmpl-3","object":"chat.completion","created":3,"model":"gpt-4o","choices":[{{"index":0,"message":{message},"finish_reason":"stop"}}]}}"#
    ))
}

fn tool_result(tool_call_id: Option<&str>, result: &str, name: Option<&str>) -> McpToolResult {
    McpToolResult {
        tool_call_id: tool_call_id.map(str::to_owned),
        result: result.to_owned(),
        name: name.map(str::to_owned),
    }
}

#[test]
fn follow_up_message_order() {
    let originals = messages(
        r#"[{"role":"user","content":null,"name":"recall"},{"role":"user","content":"weather in Oslo?"}]"#,
    );
    let unchanged = originals.clone();
    let results = [
        tool_result(Some("call_1"), "sunny", Some("lookup")),
        tool_result(None, "unknown", None),
    ];
    let follow_up =
        create_follow_up_messages_for_chat(&originals, &response(TOOL_CALLS_RESPONSE), &results)
            .expect("the follow-up messages convert");
    assert_eq!(originals, unchanged);
    assert_eq!(
        follow_up,
        messages(
            r#"[
                {"role":"user","name":"recall"},
                {"role":"user","content":"weather in Oslo?"},
                {"role":"assistant","tool_calls":[
                    {"id":"call_1","type":"function","function":{"name":"lookup","arguments":""}},
                    {"id":"call_2","type":"function","function":{"name":"forecast","arguments":"{\"city\":\"Oslo\"}"}}
                ],"refusal":"no"},
                {"role":"tool","tool_call_id":"call_1","name":"lookup","content":"sunny"},
                {"role":"tool","tool_call_id":null,"name":null,"content":"unknown"}
            ]"#
        )
    );
}

#[rstest]
#[case::null_content(r#"{"role":"assistant","content":null}"#, r#"{"role":"assistant"}"#)]
#[case::no_tool_calls(
    r#"{"role":"assistant","content":"hello"}"#,
    r#"{"role":"assistant","content":"hello"}"#
)]
#[case::empty_tool_calls(
    r#"{"role":"assistant","content":"hello","tool_calls":[]}"#,
    r#"{"role":"assistant","content":"hello"}"#
)]
#[case::empty_arguments(
    r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":""}}]}"#,
    r#"{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":""}}]}"#
)]
#[case::annotations(
    r#"{"role":"assistant","content":"cited","annotations":[{"type":"url_citation","url":"https://example.com","title":null,"meta":{"a":null}}]}"#,
    r#"{"role":"assistant","content":"cited","annotations":[{"type":"url_citation","url":"https://example.com","meta":{"a":null}}]}"#
)]
#[case::empty_annotations(
    r#"{"role":"assistant","content":"cited","annotations":[]}"#,
    r#"{"role":"assistant","content":"cited","annotations":[]}"#
)]
#[case::audio(
    r#"{"role":"assistant","content":null,"audio":{"id":"audio_1","data":"ZGF0YQ==","expires_at":1,"transcript":"hi","extra":null}}"#,
    r#"{"role":"assistant","audio":{"id":"audio_1","data":"ZGF0YQ==","expires_at":1,"transcript":"hi"}}"#
)]
#[case::refusal(
    r#"{"role":"assistant","content":null,"refusal":"declined"}"#,
    r#"{"role":"assistant","refusal":"declined"}"#
)]
fn assistant_dump_drops_nulls(#[case] message: &str, #[case] expected: &str) {
    let follow_up = create_follow_up_messages_for_chat(&[], &response_with_message(message), &[])
        .expect("the assistant message converts");
    assert_eq!(follow_up, vec![json(expected)]);
}

#[test]
fn empty_choices_warning() {
    let (guard, captured) = capture();
    let originals = messages(r#"[{"role":"user","content":"search the docs"}]"#);
    let results = [tool_result(Some("call_9"), "found", Some("search"))];
    let follow_up =
        create_follow_up_messages_for_chat(&originals, &response(EMPTY_RESPONSE), &results)
            .expect("the follow-up messages convert");
    drop(guard);
    assert_eq!(lines(&captured), vec![String::from(CONVERSION_FAILED)]);
    assert_eq!(
        follow_up,
        messages(
            r#"[
                {"role":"user","content":"search the docs"},
                {"role":"tool","tool_call_id":"call_9","name":"search","content":"found"}
            ]"#
        )
    );
}

#[test]
fn non_object_message_error() {
    let originals = [Json::Text(String::from("not a message object"))];
    let error = create_follow_up_messages_for_chat(&originals, &response(TOOL_CALLS_RESPONSE), &[])
        .expect_err("a string message fails the conversion");
    assert!(matches!(
        error,
        ChatFollowupError::InvalidMessages(ValidationError::NotAnObject)
    ));
}
