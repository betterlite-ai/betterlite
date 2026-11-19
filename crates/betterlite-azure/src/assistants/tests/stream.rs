#![cfg(test)]

use std::time::Duration;

use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::thread_runs::records::RunEvent;

use super::support::{self, CREATE_URL, KEY, Recorder, STREAM_BODY, run_body, run_request};
use crate::assistants::operations::RunDispatch;

// A streaming facade call hands back the SDK's manager, which sends its request only when read.
pub(super) async fn check() {
    builds_without_sending().await;
    streams_events_in_order().await;
}

fn streaming() -> RunDispatch {
    RunDispatch {
        arun_thread: None,
        stream: Some(true),
    }
}

async fn builds_without_sending() {
    let recorder = Recorder::start();
    let fixture = support::Fixture::new();
    let manager = support::streamed(&recorder, &fixture, &run_request(), streaming(), None).await;
    assert_eq!(manager.kind, ClientKind::Sync);
    assert_eq!(manager.read_timeout, Duration::from_secs(600));
    assert_eq!(manager.request.method.as_str(), "POST");
    assert_eq!(manager.request.url.as_str(), CREATE_URL);
    assert_eq!(manager.request.deadline, Duration::from_mins(100));
    assert!(manager.handler.is_none());
    assert!(recorder.requests().is_empty());
}

async fn streams_events_in_order() {
    let recorder = Recorder::start();
    recorder.queue(200, STREAM_BODY);
    let fixture = support::Fixture::new();
    let manager = support::streamed(&recorder, &fixture, &run_request(), streaming(), None).await;
    let events = manager.consume().await.unwrap();
    let names: Vec<&str> = events.iter().map(RunEvent::name).collect();
    assert_eq!(names, ["thread.run.created", "thread.run.completed"]);
    let seen = recorder.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, CREATE_URL);
    assert_eq!(seen[0].body, Some(run_body(true)));
    assert_eq!(seen[0].deadline, Duration::from_mins(100));
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some(KEY)
    );
}
