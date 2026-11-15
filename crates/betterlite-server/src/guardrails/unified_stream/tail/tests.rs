#![cfg(test)]

use std::collections::BTreeMap;

use crate::guardrails::unified_stream::tests::support::{
    FakeGuardrail, FakeTranslation, Plan, collect_outputs, item, metadata_chunk, params, synthetic,
    text_chunk,
};

#[tokio::test]
async fn tail_forces_holdback_to_zero_on_the_final_round() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "abcdef",
            holdback: 3,
        },
        Plan::Rewrite {
            index: 0,
            text: "abcdef",
            holdback: 3,
        },
        Plan::Rewrite {
            index: 0,
            text: "abcdef",
            holdback: 3,
        },
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a"), text_chunk("b")]).await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("abc", Some("assistant"), None))),
            Ok(item(synthetic("def", None, None))),
        ]
    );
}

#[tokio::test]
async fn tail_replays_trailing_metadata_after_the_final_round() {
    let translation = FakeTranslation::planned(vec![Plan::Rewrite {
        index: 0,
        text: "hi",
        holdback: 0,
    }]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("hi"), metadata_chunk()],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("hi", Some("assistant"), None))),
            Ok(item(metadata_chunk())),
        ]
    );
}

#[tokio::test]
async fn tail_replays_metadata_seen_before_text_too() {
    let translation = FakeTranslation::planned(vec![Plan::Rewrite {
        index: 0,
        text: "hi",
        holdback: 0,
    }]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![metadata_chunk(), text_chunk("hi")],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("hi", Some("assistant"), None))),
            Ok(item(metadata_chunk())),
        ]
    );
}

#[tokio::test]
async fn tail_without_a_reference_chunk_only_replays_metadata() {
    let translation = FakeTranslation::planned(Vec::new());
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![metadata_chunk()],
    )
    .await;
    assert_eq!(outputs, vec![Ok(item(metadata_chunk()))]);
    assert!(translation.calls().is_empty());
}

#[tokio::test]
async fn tail_does_not_use_a_metadata_chunk_as_the_reference() {
    let translation = FakeTranslation::planned(vec![
        Plan::Pass,
        Plan::Rewrite {
            index: 0,
            text: "ab",
            holdback: 0,
        },
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a"), metadata_chunk()]).await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("ab", Some("assistant"), None))),
            Ok(item(metadata_chunk())),
        ]
    );
}
