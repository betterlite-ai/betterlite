#![cfg(test)]

use std::time::Duration;

use betterlite_openai::thread_runs::records::RunRequest;

use super::support::{
    self, COMPLETED, CREATED, ERROR_BODY, FAILED, REQUIRES_ACTION, Recorder, parsed, run_request,
    status_of,
};

// The run keywords override the assistant's stored defaults, and every result comes back untouched.
pub(super) async fn check() {
    overrides_the_assistant_defaults().await;
    preserves_terminal_results().await;
    passes_sdk_errors_through().await;
    keeps_the_callers_deadline_per_request().await;
    merges_a_base_url_query().await;
    drops_an_empty_query_value().await;
}

// The SDK's query builder leaves out a parameter with an empty value, so a base blank never rides along.
async fn drops_an_empty_query_value() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let mut fixture = support::Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?drop=&api-version=old",
    ));
    support::polled(&recorder, &fixture, &run_request()).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        "https://example.test/openai/deployments/dep?api-version=2024-02-01"
    );
    assert_eq!(
        seen[1].url,
        "https://example.test/openai/deployments/dep?api-version=2024-02-01"
    );
}

// The SDK joins the tail onto the raw URL, so a base query keeps the tail in its last value and api-version wins.
async fn merges_a_base_url_query() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let mut fixture = support::Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?api-version=old&empty=",
    ));
    support::polled(&recorder, &fixture, &run_request()).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        "https://example.test/openai/deployments/dep?api-version=2024-02-01&empty=%2Fthreads%2Fthread_1%2Fruns"
    );
    assert_eq!(
        seen[1].url,
        "https://example.test/openai/deployments/dep?api-version=2024-02-01&empty=%2Fthreads%2Fthread_1%2Fruns%2Frun_1"
    );
}

async fn overrides_the_assistant_defaults() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let run = RunRequest {
        thread_id: String::from("thread_1"),
        assistant_id: String::from("asst_1"),
        additional_instructions: Some(String::from("extra")),
        instructions: Some(String::from("override")),
        metadata: Some(parsed(r#"{"tag":"x"}"#)),
        model: Some(String::from("gpt-4o")),
        tools: Some(parsed(r#"[{"type":"code_interpreter"}]"#)),
    };
    support::polled(&recorder, &fixture, &run).await;
    let seen = recorder.requests();
    assert_eq!(
        seen[0].body,
        Some(parsed(concat!(
            r#"{"assistant_id":"asst_1","additional_instructions":"extra","#,
            r#""instructions":"override","metadata":{"tag":"x"},"model":"gpt-4o","#,
            r#""stream":false,"tools":[{"type":"code_interpreter"}]}"#,
        )))
    );
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].body, None);
    assert_eq!(seen[1].headers.get("content-type"), None);
}

async fn preserves_terminal_results() {
    for body in [REQUIRES_ACTION, FAILED, COMPLETED] {
        let recorder = Recorder::start();
        recorder.queue_poll(200, CREATED);
        recorder.queue_poll(200, body);
        let fixture = support::Fixture::new();
        let (_, reply) = support::polled(&recorder, &fixture, &run_request()).await;
        match reply.status.as_str() {
            "requires_action" => assert_eq!(
                reply.required_action,
                Some(parsed(
                    r#"{"type":"submit_tool_outputs","submit_tool_outputs":{"tool_calls":[]}}"#
                ))
            ),
            "failed" => assert_eq!(
                reply.last_error,
                Some(parsed(r#"{"code":"server_error","message":"boom"}"#))
            ),
            _ => assert_eq!(
                reply.usage,
                Some(parsed(
                    r#"{"completion_tokens":10,"prompt_tokens":5,"total_tokens":15}"#
                ))
            ),
        }
    }
}

async fn passes_sdk_errors_through() {
    let recorder = Recorder::start();
    recorder.queue(400, ERROR_BODY);
    let fixture = support::Fixture::new();
    let error = support::failed(&recorder, &fixture, &run_request()).await;
    let (status, body) = status_of(error);
    assert_eq!(status, 400);
    assert_eq!(body, ERROR_BODY.as_bytes());
}

async fn keeps_the_callers_deadline_per_request() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let mut fixture = support::Fixture::new();
    fixture.params.timeout = Some(30.0_f64);
    support::polled(&recorder, &fixture, &run_request()).await;
    let seen = recorder.requests();
    assert_eq!(seen.len(), 3);
    for request in &seen {
        assert_eq!(request.deadline, Duration::from_secs(30));
    }
}
