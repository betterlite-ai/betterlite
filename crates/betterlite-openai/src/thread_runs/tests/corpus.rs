#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::thread_runs::decode::decode_run_event;
use crate::thread_runs::error::ThreadRunsError;

use super::support::QUEUED;

// Each SSE run event name must keep its own variant; a swap would still decode, so assert the name comes back.
#[test]
fn every_run_event_name_keeps_its_variant() {
    [
        "thread.run.created",
        "thread.run.queued",
        "thread.run.in_progress",
        "thread.run.requires_action",
        "thread.run.completed",
        "thread.run.incomplete",
        "thread.run.failed",
        "thread.run.cancelling",
        "thread.run.cancelled",
        "thread.run.expired",
    ]
    .iter()
    .for_each(|name| {
        let frame = format!("event: {name}\ndata: {QUEUED}");
        let Some(event) = decode_run_event(frame.as_bytes()).unwrap() else {
            panic!("{name} ended the stream");
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value.get("event"),
            Some(&Value::String(String::from(*name)))
        );
    });
}

// The file extension names the outcome a seed must produce.
#[test]
fn replays_the_openai_thread_runs_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_thread_runs");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if name.starts_with('.') {
                return Ok(());
            }
            let bytes = fs::read(&path)?;
            let outcome = match decode_run_event(&bytes) {
                Ok(Some(_event)) => "ok",
                Ok(None) => "done",
                Err(ThreadRunsError::Truncated { .. }) => "truncated",
                Err(ThreadRunsError::Malformed { .. }) => "malformed",
                Err(ThreadRunsError::Shape { .. }) => "shape",
                Err(ThreadRunsError::UnsupportedEvent { .. }) => "unsupported",
                Err(other) => panic!("{name} failed outside decoding: {other}"),
            };
            assert_eq!(
                path.extension().unwrap().to_str().unwrap(),
                outcome,
                "{name}"
            );
            replayed += 1;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed >= 8, "{replayed} seeds replayed");
}
