#![cfg(test)]

use std::sync::Arc;

use tokio_util::task::TaskTracker;

use super::support::{CountingSource, RewriteAll, timed_scan};
use crate::api::file_batch_guardrails::REWRITE_SPOOL_BYTES;
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
    let mut source = CountingSource::new(rows.join("\n"));
    let spool = Spool::default();
    let outcome = timed_scan(&mut source, &RewriteAll, &spool, &tasks)
        .await
        .expect("a scan answers");
    let ScanFile::Scanned(result) = outcome else {
        panic!("every record is rewritten, not refused");
    };
    assert_eq!(result.changes.len(), 32);
    assert!(result.redactions.is_rolled());
}

async fn append(tasks: &TaskTracker, spool: &Spool, bytes: Vec<u8>) -> (u64, u64) {
    let length = u64::try_from(bytes.len()).unwrap();
    let offset = spool.append(tasks, bytes).await.unwrap();
    (offset, length)
}
