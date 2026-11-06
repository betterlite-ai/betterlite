#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::thread_runs::decode::decode_run_event;
use crate::thread_runs::error::ThreadRunsError;

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
