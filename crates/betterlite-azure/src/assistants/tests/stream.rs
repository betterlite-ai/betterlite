#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::thread_runs::records::RunEvent;

use super::support::{self, CREATE_URL, KEY, Recorder, STREAM_BODY, run_body, run_request};
use crate::assistants::error::AssistantsRunError;
use crate::assistants::operations::{RunCall, RunDispatch};
use crate::azure_assistant_definitions::error::DefinitionsError;

// A streaming facade call hands back the SDK's manager, which sends its request only when read.
pub(super) async fn check() {
    builds_without_sending().await;
    streams_events_in_order().await;
    resolves_the_provider_when_the_manager_is_built().await;
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

// The manager's request, and the provider token in it, are fixed when the manager is built, not when it opens.
async fn resolves_the_provider_when_the_manager_is_built() {
    let mut fixture = support::Fixture::new();
    fixture.provider = Some(support::provider(Arc::new(support::FailingToken)));
    let recorder = Recorder::start();
    let run = run_request();
    let call = RunCall {
        dispatch: streaming(),
        run: &run,
        handler: None,
    };
    let outcome = support::api(&recorder)
        .run(&fixture.request(), None, call)
        .await;
    assert!(matches!(
        outcome,
        Err(AssistantsRunError::Client(
            DefinitionsError::Provider { .. }
        ))
    ));
    assert!(recorder.requests().is_empty());
}
