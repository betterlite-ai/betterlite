#![cfg(test)]

use super::support::{
    Harness, SESSION, content_texts, data_messages, field_of, inputs, masked, passing, records_of,
    strings, text, texts_of,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_scan_cache::scanned_text_hash;

#[tokio::test]
async fn only_the_new_segments_reach_bedrock() {
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
    assert_eq!(
        field_of(&harness.transport.bodies()[0], "source"),
        text("INPUT")
    );
    let records = records_of(&harness.data);
    assert_eq!(records.len(), 1);
    assert_eq!(field_of(&records[0], "guardrail_mode"), text("pre_call"));
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
    assert_eq!(
        harness.cache.hashes(SESSION),
        vec![scanned_text_hash("hello"), scanned_text_hash("world")]
    );
}

#[tokio::test]
async fn latest_role_only_does_not_narrow_the_incremental_scan() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.latest_user_only();
    harness.cache.remember(SESSION, &["sys"]);
    harness.data.insert(
        "messages".to_owned(),
        data_messages(
            r#"[
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "first"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": "second"}
            ]"#,
        ),
    );

    let result = harness
        .apply(
            inputs(&["sys", "first", "reply", "second"]),
            GuardrailInputType::Request,
        )
        .await
        .expect("the incremental scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["first", "reply", "second"])
    );
    assert_eq!(
        texts_of(&result),
        strings(&["sys", "first", "reply", "second"])
    );
}

#[tokio::test]
async fn no_new_segments_skip_the_bedrock_call() {
    let mut harness = Harness::new(vec![]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let result = harness
        .apply(inputs(&["hello"]), GuardrailInputType::Request)
        .await
        .expect("the skip is not a failure");

    assert_eq!(harness.transport.count(), 0);
    assert!(harness.cache.ttls().is_empty());
    assert_eq!(texts_of(&result), strings(&["hello"]));
}

#[tokio::test]
async fn a_masked_incremental_result_forces_a_full_scan() {
    let mut harness = Harness::new(vec![masked("MASKED"), passing()]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the fallback full scan passes");

    assert_eq!(harness.transport.count(), 2);
    let bodies = harness.transport.bodies();
    assert_eq!(content_texts(&bodies[0]), strings(&["world"]));
    assert_eq!(content_texts(&bodies[1]), strings(&["hello", "world"]));
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
    assert_eq!(
        harness.cache.hashes(SESSION),
        vec![scanned_text_hash("hello")]
    );
}

#[tokio::test]
async fn a_response_scan_ignores_the_session_cache() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.cache.remember(SESSION, &["hello"]);

    let result = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Response)
        .await
        .expect("the output scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["hello", "world"])
    );
    assert!(harness.cache.ttls().is_empty());
    assert_eq!(texts_of(&result), strings(&["hello", "world"]));
}

#[tokio::test]
async fn a_masking_guardrail_scans_the_full_context() {
    let mut harness = Harness::new(vec![passing()]).with_session();
    harness.incremental();
    harness.base.settings.mask_request_content = true;
    harness.cache.remember(SESSION, &["hello"]);

    let _applied = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the full scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["hello", "world"])
    );
}

#[tokio::test]
async fn a_session_less_request_scans_the_full_context() {
    let mut harness = Harness::new(vec![passing()]);
    harness.incremental();

    let _applied = harness
        .apply(inputs(&["hello", "world"]), GuardrailInputType::Request)
        .await
        .expect("the full scan passes");

    assert_eq!(harness.transport.count(), 1);
    assert_eq!(
        content_texts(&harness.transport.bodies()[0]),
        strings(&["hello", "world"])
    );
}
