#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::client_kind::ClientKind;
use http::{HeaderMap, HeaderName, HeaderValue};
use tokio::time::Instant;

use super::support::{
    self, COMPLETED, CREATE_URL, CREATED, KEY, POLL_URL, Recorder, run_body, run_request,
};

// The facade's default call creates the run, then retrieves it until the SDK's terminal set matches.
pub(super) async fn check() {
    creates_then_polls().await;
    create_status_does_not_end_the_wait().await;
    stops_at_every_terminal_state().await;
    reuses_the_first_interval().await;
    defaults_to_one_second().await;
    fills_the_declared_model_keys().await;
}

async fn creates_then_polls() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let (kind, reply) = support::polled(&recorder, &fixture, &run_request()).await;
    assert_eq!(kind, ClientKind::Sync);
    assert_eq!(reply.id, "run_1");
    assert_eq!(reply.status, "completed");
    assert_eq!(reply.instructions, "base");
    assert_eq!(reply.model, "gpt-4o-mini");
    let seen = recorder.requests();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, CREATE_URL);
    assert_eq!(seen[0].body, Some(run_body(false)));
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some(KEY)
    );
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, POLL_URL);
    assert_eq!(
        seen[1].headers.get("api-key").map(String::as_str),
        Some(KEY)
    );
    assert_eq!(seen[1].body, None);
}

async fn create_status_does_not_end_the_wait() {
    let recorder = Recorder::start();
    let mut done = support::parsed(CREATED);
    done["status"] = JsonValue::String(String::from("completed"));
    recorder.queue(200, &done.to_string());
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let (_, reply) = support::polled(&recorder, &fixture, &run_request()).await;
    assert_eq!(reply.status, "completed");
    assert_eq!(recorder.requests().len(), 3);
}

async fn stops_at_every_terminal_state() {
    let states = [
        "requires_action",
        "cancelled",
        "completed",
        "failed",
        "expired",
        "incomplete",
    ];
    for status in states {
        let recorder = Recorder::start();
        recorder.queue_poll(200, CREATED);
        let mut fetched = support::parsed(CREATED);
        fetched["status"] = JsonValue::String(status.to_owned());
        recorder.queue_poll(200, &fetched.to_string());
        let fixture = support::Fixture::new();
        let (_, reply) = support::polled(&recorder, &fixture, &run_request()).await;
        assert_eq!(reply.status, status);
        assert_eq!(recorder.requests().len(), 2);
    }
}

async fn reuses_the_first_interval() {
    let recorder = Recorder::start();
    let mut moving = support::parsed(CREATED);
    moving["status"] = JsonValue::String(String::from("in_progress"));
    recorder.queue(200, CREATED);
    let mut first = HeaderMap::new();
    first.insert(
        HeaderName::from_static("openai-poll-after-ms"),
        HeaderValue::from_static("0"),
    );
    recorder.queue_with(200, first, &moving.to_string());
    let mut later = HeaderMap::new();
    later.insert(
        HeaderName::from_static("openai-poll-after-ms"),
        HeaderValue::from_static("7000"),
    );
    recorder.queue_with(200, later, &moving.to_string());
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let started = Instant::now();
    let (_, reply) = support::polled(&recorder, &fixture, &run_request()).await;
    assert_eq!(reply.status, "completed");
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(recorder.requests().len(), 4);
}

const SHAPED: &str = concat!(
    r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","#,
    r#""thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","#,
    r#""parallel_tool_calls":true,"tool_choice":{"type":"function"},"incomplete_details":{"extra":1},"#,
    r#""truncation_strategy":{"type":"auto"},"#,
    r#""response_format":{"type":"json_schema","json_schema":{"name":"reply"}},"#,
    r#""tools":[{"type":"file_search"},"#,
    r#"{"type":"file_search","file_search":{"max_num_results":2}},"#,
    r#"{"type":"function","function":{"name":"lookup"}}]}"#,
);

// The reply shows the fields the SDK's Run model declares, so a nested model keeps its missing keys as null.
async fn fills_the_declared_model_keys() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, SHAPED);
    let fixture = support::Fixture::new();
    let (_, reply) = support::polled(&recorder, &fixture, &run_request()).await;
    assert_eq!(
        reply.truncation_strategy,
        Some(support::parsed(r#"{"type":"auto","last_messages":null}"#))
    );
    assert_eq!(
        reply.tool_choice,
        Some(support::parsed(r#"{"type":"function","function":null}"#))
    );
    assert_eq!(
        reply.incomplete_details,
        Some(support::parsed(r#"{"extra":1,"reason":null}"#))
    );
    assert_eq!(
        reply.response_format,
        Some(support::parsed(
            r#"{"type":"json_schema","json_schema":{"name":"reply","description":null,"strict":null,"schema_":null}}"#
        ))
    );
    assert_eq!(
        reply.tools,
        vec![
            support::parsed(r#"{"type":"file_search","file_search":null}"#),
            support::parsed(
                r#"{"type":"file_search","file_search":{"max_num_results":2,"ranking_options":null}}"#
            ),
            support::parsed(
                r#"{"type":"function","function":{"name":"lookup","description":null,"parameters":null,"strict":null}}"#
            ),
        ]
    );
}

async fn defaults_to_one_second() {
    let recorder = Recorder::start();
    let mut moving = support::parsed(CREATED);
    moving["status"] = JsonValue::String(String::from("in_progress"));
    recorder.queue(200, CREATED);
    recorder.queue(200, &moving.to_string());
    recorder.queue_poll(200, COMPLETED);
    let fixture = support::Fixture::new();
    let started = Instant::now();
    support::polled(&recorder, &fixture, &run_request()).await;
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}
