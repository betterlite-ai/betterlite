#![cfg(test)]

use chrono::{TimeZone, Utc};
use serde_json::{Map, Value, json};

use super::super::mlflow::{Kwargs, MlflowLogger, RecordingMlflowClient};

fn times() -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    (
        Utc.timestamp_opt(1, 0).single().expect("valid timestamp"),
        Utc.timestamp_opt(2, 0).single().expect("valid timestamp"),
    )
}

#[test]
fn non_streaming_success_ends_trace_and_extracts_chat_data() {
    let client = RecordingMlflowClient::default();
    let calls = client.calls.clone();
    let logger = MlflowLogger::new(client);
    let mut kwargs: Kwargs = Map::from_iter([
        (
            "messages".to_owned(),
            json!([{ "role": "user", "content": "hi" }]),
        ),
        ("call_type".to_owned(), json!("completion")),
        ("request_tags".to_owned(), json!(["team: core", "audit"])),
    ]);
    let (start, end) = times();
    logger.log_success_event(
        &mut kwargs,
        &json!({ "choices": [{ "message": { "role": "assistant", "content": "hello" } }] }),
        start,
        end,
    );
    let recorded = calls.lock().expect("recording lock").clone();
    assert_eq!(
        recorded,
        vec!["trace:litellm-completion", "chat_messages", "end_trace"]
    );
}

#[test]
fn streamed_chunks_share_span_and_final_chunk_cleans_up() {
    let client = RecordingMlflowClient::default();
    let calls = client.calls.clone();
    let logger = MlflowLogger::new(client);
    let (start, end) = times();
    let mut first = stream_kwargs(false);
    logger.log_success_event(
        &mut first,
        &json!({ "choices": [{ "delta": { "content": "a" } }] }),
        start,
        end,
    );
    let mut final_chunk = stream_kwargs(true);
    logger.log_success_event(
        &mut final_chunk,
        &json!({ "choices": [{ "delta": { "content": "b" } }] }),
        start,
        end,
    );
    let recorded = calls.lock().expect("recording lock").clone();
    assert_eq!(
        recorded,
        vec![
            "trace:litellm-completion",
            "streaming_chunk",
            "streaming_chunk",
            "end_trace"
        ]
    );
}

fn stream_kwargs(final_chunk: bool) -> Kwargs {
    let mut kwargs = Kwargs::from_iter([
        ("stream".to_owned(), Value::Bool(true)),
        ("litellm_call_id".to_owned(), json!("call-1")),
    ]);
    if final_chunk {
        kwargs.insert(
            "complete_streaming_response".to_owned(),
            json!({ "choices": [] }),
        );
    }
    kwargs
}
