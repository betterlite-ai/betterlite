#![cfg(test)]

use std::fs;
use std::sync::Arc;

use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::gemini_live_content::map::{MappedEvent, map_model_turn_event};

fn warnings_of<Run>(run: Run) -> String
where
    Run: FnOnce(),
{
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("the log reopens")))
        .finish();
    with_default(subscriber, run);
    fs::read_to_string(log.path()).expect("the log reads")
}

#[test]
fn warns_only_on_multi_part_turns() {
    let single = serde_json::from_str(r#"{"parts": [{"text": "hi"}]}"#).expect("the turn parses");
    let single_log = warnings_of(|| {
        assert_eq!(
            map_model_turn_event(&single).expect("a single part maps"),
            MappedEvent::TextDelta
        );
    });
    assert!(!single_log.contains("Expected 1 part"));
    let many = serde_json::from_str(r#"{"parts": [{"text": "a"}, {"text": "b"}]}"#)
        .expect("the turn parses");
    let many_log = warnings_of(|| {
        assert_eq!(
            map_model_turn_event(&many).expect("the first part maps"),
            MappedEvent::TextDelta
        );
    });
    assert!(many_log.contains("Realtime: Expected 1 part, got 2 for Gemini model turn event."));
}
