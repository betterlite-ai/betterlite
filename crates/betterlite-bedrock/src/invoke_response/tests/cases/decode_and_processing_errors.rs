#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::invoke_response::tests::counter::FixtureCounter;
use crate::invoke_response::tests::support::{MODEL, decode, decode_with, fixture};

pub(in crate::invoke_response::tests) fn decode_and_processing_errors() {
    decode_failures_keep_the_raw_response();
    invalid_utf8_becomes_replacement_characters();
    malformed_family_payloads_are_processing_errors();
    debug_log_carries_the_rendered_reply();
}

fn decode_failures_keep_the_raw_response() {
    let mut case = fixture();
    case.headers = vec![(String::from("x-amzn-requestid"), String::from("req-1"))];
    let invalid_run = decode(&case, "not json");
    assert_eq!(invalid_run.status(), Some(200));
    assert_eq!(invalid_run.message(), "not json");
    assert_eq!(invalid_run.response_headers(), case.headers.as_slice());

    case.status = 400;
    let truncated = r#"{"results": [{"outputText": "cut"#;
    let truncated_run = decode(&case, truncated);
    assert_eq!(truncated_run.status(), Some(400));
    assert_eq!(truncated_run.message(), truncated);
}

fn invalid_utf8_becomes_replacement_characters() {
    let case = fixture();
    let counter = FixtureCounter::counting(1, 1);
    let run = decode_with(&case, b"{\"results\": [\xff]}", &counter);
    assert_eq!(run.status(), Some(200));
    assert_eq!(run.message(), "{\"results\": [\u{FFFD}]}");
}

fn malformed_family_payloads_are_processing_errors() {
    let cases = [
        (
            MODEL,
            r#"{"other": 1}"#,
            "the results field is missing or malformed",
        ),
        (
            MODEL,
            r#"{"results": 5}"#,
            "the results field is missing or malformed",
        ),
        (
            MODEL,
            r#"{"results": []}"#,
            "the results field is missing or malformed",
        ),
        (MODEL, r#"[1, 2]"#, "the response body is not a JSON object"),
        (
            "ai21.j2-mid-v1",
            r#"{}"#,
            "the completions field is missing or malformed",
        ),
        (
            "meta.llama3-70b-instruct-v1:0",
            r#"{}"#,
            "the generation field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": []}"#,
            "the generations field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": [{"finish_reason": "COMPLETE"}]}"#,
            "the generations.text field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": [{"text": "x", "finish_reason": ["a"]}]}"#,
            "the generations.finish_reason field has a type its declared shape does not allow",
        ),
        (
            "mistral.mistral-large-2402-v1:0",
            r#"{"wrong": true}"#,
            "Unexpected mistral completion response",
        ),
        (
            "mistral.mistral-large-2402-v1:0",
            r#"{"choices": [{"message": {"content": "x"}}]}"#,
            "the choices.finish_reason field is missing or malformed",
        ),
    ];
    for (model, payload, expected) in cases {
        let mut case = fixture();
        case.model = model.to_owned();
        let run = decode(&case, payload);
        assert_eq!(run.status(), Some(422), "{model}: {payload}");
        assert_eq!(
            run.message(),
            format!("Error processing={payload}, Received error={expected}"),
            "{model}: {payload}"
        );
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Captured {
    fn logged(&self) -> Vec<String> {
        let messages = self.0.lock().expect("the capture lock is not poisoned");
        messages
            .iter()
            .filter(|message| message.contains("bedrock invoke response"))
            .cloned()
            .collect()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .expect("the capture lock is not poisoned")
            .push(message.0);
    }
}

fn debug_log_carries_the_rendered_reply() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let guard = tracing::subscriber::set_default(subscriber);
    let _run = decode(&fixture(), r#"{"results": [{"outputText": "the answer"}]}"#);
    drop(guard);
    let messages = captured.logged();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("outputText"));
    assert!(messages[0].contains("the answer"));
    assert!(
        messages[0].contains(
            "\n    \"results\": [\n        {\n            \"outputText\": \"the answer\"\n        }\n    ]\n}"
        ),
        "the reply is logged with indent=4"
    );
}
