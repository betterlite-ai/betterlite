#![cfg(test)]

use super::support::{Harness, SESSION, content_texts, inputs, passing, strings, texts_of};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_scan_cache::scanned_text_hash;

#[tokio::test]
async fn a_seeded_cache_keeps_the_scanned_segments_out_of_the_scan() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the incremental scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["world"])
    );
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}

#[tokio::test]
async fn a_successful_scan_marks_the_texts_with_the_configured_ttl() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.ttl_seconds = 120;

    let _applied = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the incremental scan passes");

    assert_eq!(harness.cache.ttls(), vec![120]);
    assert_eq!(
        harness.cache.hashes(SESSION),
        vec![scanned_text_hash("hello")]
    );
}

#[tokio::test]
async fn an_unreadable_cache_falls_back_to_a_full_scan() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.cache.fail_reads = true;

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the full scan passes");

    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["hello", "world"])
    );
    assert!(harness.cache.ttls().is_empty());
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}
