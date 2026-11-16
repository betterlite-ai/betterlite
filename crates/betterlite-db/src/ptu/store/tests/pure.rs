use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::ptu::store::StoreError;
use crate::ptu::store::prune::{prune_chunks, prune_cutoff};

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn ids(range: std::ops::Range<u32>) -> BTreeSet<String> {
    range.map(|index| format!("model-{index:04}")).collect()
}

#[test]
fn chunks_hold_five_thousand_sorted_ids() {
    let scanned = ids(0..5001);
    let cutoff = instant("2026-03-01T11:55:00Z");
    let chunks = prune_chunks("2026-03-01", cutoff, &scanned);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].models.len(), 5000);
    assert_eq!(chunks[1].models, ["model-5000"]);
    assert_eq!(chunks[0].models.first().unwrap(), "model-0000");
    assert_eq!(chunks[0].models.last().unwrap(), "model-4999");
}

#[test]
fn every_chunk_repeats_the_date_and_cutoff() {
    let scanned = ids(0..3);
    let cutoff = instant("2026-03-01T11:55:00Z");
    let chunks = prune_chunks("2026-03-01", cutoff, &scanned);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].date, "2026-03-01");
    assert_eq!(chunks[0].cutoff, cutoff);
    assert_eq!(chunks[0].models, ["model-0000", "model-0001", "model-0002"]);
}

#[test]
fn an_empty_scan_makes_no_chunks() {
    let chunks = prune_chunks(
        "2026-03-01",
        instant("2026-03-01T11:55:00Z"),
        &BTreeSet::new(),
    );
    assert!(chunks.is_empty());
}

#[test]
fn the_cutoff_pulls_back_the_grace() {
    let cutoff = prune_cutoff(instant("2026-03-01T12:00:00Z")).unwrap();
    assert_eq!(cutoff, instant("2026-03-01T11:55:00Z"));
}

#[test]
fn the_first_representable_instant_cannot_pull_back() {
    let result = prune_cutoff(DateTime::<Utc>::MIN_UTC);
    assert!(matches!(result, Err(StoreError::GraceUnderflow { .. })));
}
