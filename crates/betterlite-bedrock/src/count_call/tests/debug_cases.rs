use betterlite_models::decisions::Json;

use crate::count_call::counter::BedrockTokenCounter;

use super::logs::{Entry, Kind, capture};
use super::support::{Reply, call, deployment, environment, fixture, json, object, user_messages};

const SUBJECTS: [&str; 7] = [
    "processing",
    "region",
    "transformed",
    "endpoint",
    "status",
    "reply",
    "final",
];

pub(crate) async fn count_debug_events() {
    success_order_and_values().await;
    transport_failure_stops_early().await;
    status_failure_logs_no_status().await;
}

fn key() -> Vec<(&'static str, Json)> {
    vec![("api_key", Json::Text("fixture-bearer".to_owned()))]
}

async fn debug_entries(reply: Reply) -> (Vec<Entry>, bool) {
    let messages = user_messages();
    let params = deployment(&key());
    let (logs, guard) = capture();
    let built = fixture(reply, environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    drop(guard);
    (logs.of(Kind::Debug), outcome.error)
}

fn value_of(entry: &Entry) -> Json {
    serde_json::from_str(entry.value.as_deref().unwrap()).unwrap()
}

async fn success_order_and_values() {
    let (entries, failed) = debug_entries(Reply::ok(r#"{"inputTokens": 42}"#)).await;
    assert!(!failed);
    let subjects: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.subject.as_deref())
        .collect();
    assert_eq!(subjects, SUBJECTS);
    assert_eq!(
        value_of(&entries[0]),
        Json::Text("anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned())
    );
    assert_eq!(value_of(&entries[1]), Json::Text("us-west-2".to_owned()));
    assert_eq!(
        value_of(&entries[2]),
        json(
            r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#
        )
    );
    assert_eq!(
        value_of(&entries[3]),
        Json::Text(
            "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
                .to_owned()
        )
    );
    assert_eq!(
        value_of(&entries[4]),
        Json::Number(serde_json::Number::from(200_u16))
    );
    assert_eq!(value_of(&entries[5]), json(r#"{"inputTokens": 42}"#));
    assert_eq!(
        value_of(&entries[6]),
        Json::Object(object(json(r#"{"input_tokens": 42}"#)))
    );
}

async fn transport_failure_stops_early() {
    stops_early(Reply::Failure, 4).await;
}

async fn status_failure_logs_no_status() {
    stops_early(Reply::status(404, "missing"), 4).await;
}

async fn stops_early(reply: Reply, expected: usize) {
    let (entries, failed) = debug_entries(reply).await;
    assert!(failed);
    let subjects: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.subject.as_deref())
        .collect();
    assert_eq!(subjects, SUBJECTS[..expected]);
}
