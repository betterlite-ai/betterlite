#![cfg(test)]

use std::io::SeekFrom;

use tokio_util::task::TaskTracker;

use super::support::{CountedBody, UnrewindableSource, scan_result};
use crate::api::file_batch_guardrails::REWRITE_SPOOL_BYTES;
use crate::api::file_batch_guardrails::records::{RecordChange, RecordDropped, RecordRedacted};
use crate::api::file_batch_rewrite::body::UploadBody;
use crate::api::file_batch_rewrite::error::RewriteError;
use crate::api::file_batch_rewrite::rewrite::rewrite_batch_input_file;
use crate::api::file_batch_rewrite::spool::Spool;

const KEPT: &str = r#"{"custom_id":"k","body":{"x": 1}}"#;
const DROPPED: &str = r#"{"custom_id":"d","body":{"messages":[]}}"#;
const REDACTED_ROW: &str = r#"{"custom_id":"r","body":{"messages":[]}}"#;
const REWRITE: &str = "{\"custom_id\":\"r\",\"body\":{\"messages\":[{\"content\":\"\u{e9}\"}]}}";

// The untouched CRLF row keeps its space and its CR, the blank and dropped rows vanish, and the redacted row comes back from the spool
#[tokio::test]
async fn rewrite_preserves_unedited() {
    let tasks = TaskTracker::new();
    let text = REWRITE.as_bytes().to_vec();
    let length = u64::try_from(text.len()).unwrap();
    let redactions = Spool::default();
    let offset = redactions.append(&tasks, text).await.unwrap();
    let result = scan_result(
        vec![dropped_change(3), redacted_change(4, offset, length)],
        4,
        redactions,
    );
    let source = CountedBody::new(format!("{KEPT}\r\n\n{DROPPED}\n{REDACTED_ROW}"));
    let output = Spool::default();
    let rewritten = rewrite_batch_input_file(&source.body, &result, &tasks, output.clone())
        .await
        .expect("the rewrite succeeds");
    assert_eq!(
        rewritten.read_to_end(&tasks).await.unwrap(),
        format!("{KEPT}\r\n{REWRITE}").into_bytes()
    );
    assert_eq!(
        result
            .redactions
            .read_at(&tasks, offset, length)
            .await
            .unwrap(),
        REWRITE.as_bytes()
    );
    assert_eq!(offset, 0);
    assert_eq!(length, u64::try_from(REWRITE.len()).unwrap());
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
    assert!(!rewritten.is_closed());
}

#[tokio::test]
async fn rewrite_of_dropped_rows_is_empty() {
    let tasks = TaskTracker::new();
    let result = scan_result(
        vec![dropped_change(1), dropped_change(2)],
        2,
        Spool::default(),
    );
    let source = CountedBody::new(format!("{KEPT}\n{KEPT}"));
    let output = Spool::default();
    let rewritten = rewrite_batch_input_file(&source.body, &result, &tasks, output)
        .await
        .expect("the rewrite succeeds");
    assert!(rewritten.read_to_end(&tasks).await.unwrap().is_empty());
    let seeks = source.seeks.lock().unwrap();
    assert_eq!(seeks.last(), Some(&SeekFrom::Start(0)));
}

#[tokio::test]
async fn a_kept_last_row_without_a_newline_is_copied() {
    let tasks = TaskTracker::new();
    let result = scan_result(Vec::new(), 1, Spool::default());
    let source = CountedBody::new(KEPT.to_owned());
    let output = Spool::default();
    let rewritten = rewrite_batch_input_file(&source.body, &result, &tasks, output)
        .await
        .expect("the rewrite succeeds");
    assert_eq!(
        rewritten.read_to_end(&tasks).await.unwrap(),
        KEPT.as_bytes()
    );
}

// A source that reads but refuses to rewind fails the rewrite with Rewind instead of handing back the written file
#[tokio::test]
async fn a_failing_rewind_replaces_the_rewritten_file() {
    let tasks = TaskTracker::new();
    let result = scan_result(Vec::new(), 1, Spool::default());
    let source = UploadBody::new(UnrewindableSource::new(KEPT.to_owned()));
    let output = Spool::default();
    let outcome = rewrite_batch_input_file(&source, &result, &tasks, output.clone()).await;
    match outcome {
        Err(RewriteError::Rewind { .. }) => (),
        other => panic!("the rewind error replaces the rewritten file, got {other:?}"),
    }
}

#[tokio::test]
async fn rewrite_output_uses_the_same_threshold() {
    let tasks = TaskTracker::new();
    let big = "a".repeat(REWRITE_SPOOL_BYTES + 1);
    let source = CountedBody::new(format!(r#"{{"custom_id":"big","body":"{big}"}}"#));
    let result = scan_result(Vec::new(), 1, Spool::default());
    let output = Spool::default();
    let rewritten = rewrite_batch_input_file(&source.body, &result, &tasks, output)
        .await
        .expect("the rewrite succeeds");
    assert!(rewritten.is_rolled());
    let bytes = rewritten.read_to_end(&tasks).await.unwrap();
    assert!(bytes.len() > REWRITE_SPOOL_BYTES);
}

fn dropped_change(line: usize) -> RecordChange {
    RecordChange::Dropped(RecordDropped {
        line_number: line,
        custom_id: Some("d".to_owned()),
        guardrail: Some("g".to_owned()),
    })
}

fn redacted_change(line: usize, offset: u64, length: u64) -> RecordChange {
    RecordChange::Redacted(RecordRedacted {
        line_number: line,
        custom_id: Some("r".to_owned()),
        offset,
        length,
    })
}
