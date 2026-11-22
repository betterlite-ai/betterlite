#![cfg(test)]

use std::sync::Arc;

use tokio_util::task::TaskTracker;

use super::support::{Answer, CountedBody, Probe, RewriteAll, ScriptedHook, timed_scan};
use crate::api::file_batch_guardrails::REWRITE_SPOOL_BYTES;
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, RecordObject};
use crate::api::file_batch_guardrails::records::{BatchScanResult, RecordChange};
use crate::api::file_batch_rewrite::rewrite::rewrite_batch_input_file;
use crate::api::file_batch_rewrite::scan::ScanFile;
use crate::api::file_batch_rewrite::spool::Spool;

// Two-byte é then one-byte x: the offsets count encoded bytes, and each span reads back exactly its own bytes
#[tokio::test]
async fn redaction_spool_offsets() {
    let tasks = TaskTracker::new();
    let spool = Spool::default();
    let accented = "\u{e9}".as_bytes().to_vec();
    let (offset, length) = append(&tasks, &spool, accented.clone()).await;
    assert_eq!((offset, length), (0, 2));
    let (later_offset, later_length) = append(&tasks, &spool, b"x".to_vec()).await;
    assert_eq!((later_offset, later_length), (2, 1));
    assert_eq!(
        spool.read_at(&tasks, offset, length).await.unwrap(),
        accented
    );
    assert_eq!(
        spool
            .read_at(&tasks, later_offset, later_length)
            .await
            .unwrap(),
        b"x"
    );
    assert!(!spool.is_rolled());
    assert_eq!(
        spool.read_at(&tasks, 0, 3).await.unwrap(),
        "\u{e9}x".as_bytes()
    );
}

#[tokio::test]
async fn spool_rolls_only_past_one_mebibyte() {
    let tasks = TaskTracker::new();
    let spool = Spool::default();
    let (offset, length) = append(&tasks, &spool, vec![b'a'; REWRITE_SPOOL_BYTES]).await;
    assert_eq!(
        (offset, length),
        (0, u64::try_from(REWRITE_SPOOL_BYTES).unwrap())
    );
    assert!(!spool.is_rolled());
    let (later_offset, _) = append(&tasks, &spool, vec![b'b']).await;
    assert_eq!(later_offset, u64::try_from(REWRITE_SPOOL_BYTES).unwrap());
    assert!(spool.is_rolled());
    assert_eq!(
        spool.read_at(&tasks, later_offset - 1, 2).await.unwrap(),
        b"ab"
    );
}

#[tokio::test]
async fn dropping_a_spool_closes_its_file() {
    let tasks = TaskTracker::new();
    let memory = Spool::default();
    let rolled = Spool::default();
    append(&tasks, &rolled, vec![b'a'; REWRITE_SPOOL_BYTES + 1]).await;
    assert!(rolled.is_rolled());
    let memory_file = Arc::downgrade(&memory.inner);
    let rolled_file = Arc::downgrade(&rolled.inner);
    drop(memory);
    drop(rolled);
    assert!(memory_file.upgrade().is_none());
    assert!(rolled_file.upgrade().is_none());

    let kept = Spool::default();
    let other = kept.clone();
    let kept_file = Arc::downgrade(&kept.inner);
    drop(kept);
    assert!(kept_file.upgrade().is_some());
    drop(other);
    assert!(kept_file.upgrade().is_none());
}

// The scan's own redaction spool is the same kind of file, so a scan of 32 large rewrites rolls it too
#[tokio::test]
async fn scan_spool_uses_the_same_threshold() {
    let tasks = TaskTracker::new();
    let pad = "p".repeat(33_500);
    let rows: Vec<String> = (1_usize..=32)
        .map(|line| {
            format!(
                r#"{{"custom_id":"r{line}","body":{{"marker":"r{line}","messages":[],"pad":"{pad}"}}}}"#
            )
        })
        .collect();
    let source = CountedBody::new(rows.join("\n"));
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &RewriteAll, &spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("every record is rewritten, not refused");
    };
    assert_eq!(result.changes.len(), 32);
    assert!(result.redactions.is_rolled());
}

const ACCENTED_ROW: &str = r#"{"custom_id":"c1","body":{"marker":"r1","messages":[]}}"#;
const KEPT_ROW: &str = r#"{"custom_id":"c2","body":{"marker":"r2","messages":[]}}"#;
// As _spool writes it: compact JSON with the record's custom_id and é as its two UTF-8 bytes
const ACCENTED_SPOOLED: &str = "{\"custom_id\":\"c1\",\"body\":{\"content\":\"\u{e9}\"}}";

fn accented() -> RecordObject {
    let mut data = RecordObject::new();
    data.insert(
        RecordText::from("content"),
        RecordJson::Text(RecordText::from("\u{e9}")),
    );
    data
}

// Scans a blank line, the rewritten é row and a kept row, so the rewritten row is spooled at its own span
async fn scanned_accented_row(tasks: &TaskTracker) -> (CountedBody, BatchScanResult<Spool>) {
    // the blank first line is never a record, but it still makes the rewritten record line 2
    let source = CountedBody::new(format!("\n{ACCENTED_ROW}\n{KEPT_ROW}"));
    let hook = ScriptedHook::with(
        vec![("r1", Answer::Replace(accented()))],
        Probe::new(Arc::clone(&source.traces.reads)),
    );
    let spool = Spool::default();
    let outcome = timed_scan(&source.body, &hook, &spool, tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("one rewritten row is a scanned file");
    };
    (source, result)
}

// The scan's RecordRedacted carries the record's physical line, custom_id and the span of the rewritten record's UTF-8 bytes
#[tokio::test]
async fn scan_spools_the_rewritten_bytes_and_their_span() {
    let tasks = TaskTracker::new();
    let (_, result) = scanned_accented_row(&tasks).await;
    assert_eq!(result.changes.len(), 1);
    let RecordChange::Redacted(change) = &result.changes[0] else {
        panic!(
            "the rewritten row is a redaction, got {:?}",
            result.changes[0]
        );
    };
    assert_eq!(change.line_number, 2);
    assert_eq!(change.custom_id.as_deref(), Some("c1"));
    assert_eq!(change.offset, 0);
    assert_eq!(
        change.length,
        u64::try_from(ACCENTED_SPOOLED.len()).unwrap()
    );
    assert!(
        ACCENTED_SPOOLED.chars().count() < ACCENTED_SPOOLED.len(),
        "the fixture's \u{e9} must make the byte length exceed the character count"
    );
    assert_eq!(
        result
            .redactions
            .read_at(&tasks, change.offset, change.length)
            .await
            .unwrap(),
        ACCENTED_SPOOLED.as_bytes()
    );
}

// The rewrite puts exactly the spooled bytes back in the rewritten row's place and keeps the untouched row
#[tokio::test]
async fn rewrite_copies_the_spooled_bytes_of_a_redacted_row() {
    let tasks = TaskTracker::new();
    let (source, result) = scanned_accented_row(&tasks).await;
    let output = Spool::default();
    let rewritten = rewrite_batch_input_file(&source.body, &result, &tasks, output)
        .await
        .expect("the rewrite succeeds");
    assert_eq!(
        rewritten.read_to_end(&tasks).await.unwrap(),
        format!("{ACCENTED_SPOOLED}\n{KEPT_ROW}").into_bytes()
    );
}

async fn append(tasks: &TaskTracker, spool: &Spool, bytes: Vec<u8>) -> (u64, u64) {
    let length = u64::try_from(bytes.len()).unwrap();
    let offset = spool.append(tasks, bytes).await.unwrap();
    (offset, length)
}
