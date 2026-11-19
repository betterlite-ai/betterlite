#![cfg(test)]

use betterlite_openai::thread_runs::records::RunRequest;
use serde_json::json;

use super::support::{self, COMPLETED, CREATED, Recorder, parsed};

// LiteLLM hands the SDK the thread id as the path and every other run keyword in the create body.
pub(super) async fn check() {
    forwards_the_run_keywords().await;
    sends_nulls_for_missing_keywords().await;
}

async fn forwards_the_run_keywords() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let run = RunRequest {
        thread_id: String::from("thread:1"),
        assistant_id: String::from("asst_9"),
        additional_instructions: Some(String::from("use the file")),
        instructions: Some(String::from("be terse")),
        metadata: Some(json!({"team": "a"})),
        model: Some(String::from("gpt-4.1")),
        tools: Some(json!([{"type": "function", "function": {"name": "lookup"}}])),
    };
    support::polled(&recorder, &fixture, &run).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        "https://example.test/openai/threads/thread:1/runs?api-version=2024-02-01"
    );
    assert_eq!(
        seen[0].body,
        Some(parsed(concat!(
            r#"{"assistant_id":"asst_9","additional_instructions":"use the file","#,
            r#""instructions":"be terse","metadata":{"team":"a"},"model":"gpt-4.1","#,
            r#""stream":false,"tools":[{"type":"function","function":{"name":"lookup"}}]}"#,
        )))
    );
    assert_eq!(seen[1].body, None);
}

async fn sends_nulls_for_missing_keywords() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let run = RunRequest {
        thread_id: String::from("thread 1"),
        assistant_id: String::from("asst_1"),
        additional_instructions: None,
        instructions: None,
        metadata: None,
        model: None,
        tools: None,
    };
    support::polled(&recorder, &fixture, &run).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        "https://example.test/openai/threads/thread%201/runs?api-version=2024-02-01"
    );
    assert_eq!(
        seen[0].body,
        Some(parsed(concat!(
            r#"{"assistant_id":"asst_1","additional_instructions":null,"instructions":null,"#,
            r#""metadata":null,"model":null,"stream":false,"tools":null}"#,
        )))
    );
    let body = seen[0].body.as_ref().unwrap();
    assert!(body.get("thread_id").is_none());
    assert_eq!(seen[1].body, None);
}
