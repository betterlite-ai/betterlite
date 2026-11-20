#![cfg(test)]

use super::support::{self, COMPLETED, CREATED, Fixture, Recorder, STREAM_BODY, run_request};
use crate::assistants::operations::RunDispatch;

// Every beta.threads.runs call sends the Assistants v2 beta header and asks for JSON.
pub(super) async fn check() {
    headers_on_every_run().await;
    headers_on_the_stream().await;
}

fn headers(seen: &support::Seen) -> Option<(&str, &str)> {
    Some((
        seen.headers.get("openai-beta")?.as_str(),
        seen.headers.get("accept")?.as_str(),
    ))
}

async fn headers_on_every_run() {
    let recorder = Recorder::start();
    recorder.queue_poll(200, CREATED);
    recorder.queue_poll(200, COMPLETED);
    let fixture = Fixture::new();
    support::polled(&recorder, &fixture, &run_request()).await;
    let seen = recorder.requests();
    for request in &seen {
        assert_eq!(
            headers(request),
            Some(("assistants=v2", "application/json"))
        );
    }
    assert_eq!(
        seen[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(seen[1].headers.get("content-type"), None);
}

async fn headers_on_the_stream() {
    let recorder = Recorder::start();
    recorder.queue(200, STREAM_BODY);
    let fixture = Fixture::new();
    let dispatch = RunDispatch {
        arun_thread: None,
        stream: Some(true),
    };
    let run = run_request();
    let manager = support::streamed(&recorder, &fixture, &run, dispatch, None).await;
    manager.consume().await.unwrap();
    let seen = recorder.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        headers(&seen[0]),
        Some(("assistants=v2", "application/json"))
    );
    assert_eq!(
        seen[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}
