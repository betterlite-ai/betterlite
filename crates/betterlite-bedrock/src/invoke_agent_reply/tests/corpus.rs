#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::invoke_agent_reply::decode_agent_trace;
use crate::invoke_agent_reply::events::AgentEvent;

#[test]
fn replays_the_invoke_agent_reply_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/invoke_agent_reply");
    let mut seeds = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let outcome = decode_agent_trace(&bytes);
            assert_eq!(outcome.is_ok(), expects_success(&name), "{name}");
            if let Ok(events) = &outcome {
                check_events(&name, events);
            }
            seeds = seeds.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seeds >= 12, "expected committed seeds, found {seeds}");
}

// A broken frame is the only category decode_agent_trace reports: swallowed frames, unknown types and a truncated tail all decode to fewer events.
fn expects_success(name: &str) -> bool {
    !name.starts_with("broken-")
}

fn check_events(name: &str, events: &[AgentEvent]) {
    match name {
        "valid-chunks" | "valid-trace-usage" => assert_eq!(events.len(), 2, "{name}"),
        "valid-unknown-events" => assert_eq!(events.len(), 1, "{name}"),
        "swallowed-exception-frame" => assert_eq!(events.len(), 1, "{name}"),
        "boundary-empty-frame" => assert!(events.is_empty(), "{name}"),
        _ => {}
    }
}
