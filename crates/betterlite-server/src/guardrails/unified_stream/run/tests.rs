#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::tests::support::{
    FakeGuardrail, FakeTranslation, Plan, SinkUse, SourceFailure, block_texts, collect_outputs,
    collect_results, item, mixed_tool_chunk, modify_response, params, synthetic, terminator,
    text_chunk, text_chunk_finishing, tool_chunk,
};

#[tokio::test]
async fn sampling_rounds_run_at_the_sampling_interval() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "ab",
            holdback: 0,
        },
        Plan::Rewrite {
            index: 0,
            text: "abcd",
            holdback: 0,
        },
        Plan::Pass,
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 2;
    let outputs = collect_outputs(
        loop_params,
        vec![
            text_chunk("a"),
            text_chunk("b"),
            text_chunk("c"),
            text_chunk("d"),
        ],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("ab", Some("assistant"), None))),
            Ok(item(synthetic("cd", None, None))),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].items.len(), 2);
    assert_eq!(calls[1].items.len(), 4);
    assert_eq!(calls[2].items.len(), 4);
}

#[tokio::test]
async fn sampling_skips_a_chunk_with_a_finish_reason() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "a",
            holdback: 0,
        },
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
    let outputs = collect_outputs(
        loop_params,
        vec![text_chunk("a"), text_chunk_finishing("b", "stop")],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("a", Some("assistant"), None))),
            Ok(item(synthetic("b", None, Some("stop")))),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].items.len(), 1);
    assert_eq!(calls[1].items.len(), 2);
}

#[tokio::test]
async fn sampling_end_of_stream_only_never_samples_before_the_end() {
    let translation = FakeTranslation::planned(vec![Plan::Rewrite {
        index: 0,
        text: "abc",
        holdback: 0,
    }]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    loop_params.end_of_stream_only = true;
    let outputs = collect_outputs(
        loop_params,
        vec![text_chunk("a"), text_chunk("b"), text_chunk("c")],
    )
    .await;
    assert_eq!(
        outputs,
        vec![Ok(item(synthetic("abc", Some("assistant"), None)))]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].items.len(), 3);
}

#[tokio::test]
async fn sampling_rate_above_the_stream_never_samples() {
    let translation = FakeTranslation::planned(vec![Plan::Pass]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("a"), text_chunk("b")],
    )
    .await;
    assert!(outputs.is_empty());
    let calls = translation.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].items.len(), 2);
}

#[tokio::test]
async fn sampling_rate_zero_never_samples_before_the_end() {
    let translation = FakeTranslation::planned(vec![Plan::Rewrite {
        index: 0,
        text: "ab",
        holdback: 0,
    }]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 0;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a"), text_chunk("b")]).await;
    assert_eq!(
        outputs,
        vec![Ok(item(synthetic("ab", Some("assistant"), None)))]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].items.len(), 2);
}

#[tokio::test]
async fn tool_call_flushes_text_before_the_passthrough() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "he",
            holdback: 0,
        },
        Plan::Pass,
        Plan::Pass,
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("he"), tool_chunk()],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("he", Some("assistant"), None))),
            Ok(item(tool_chunk())),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].sink, SinkUse::Empty);
    assert_eq!(calls[0].items.len(), 2);
    assert_eq!(calls[1].sink, SinkUse::Absent);
    assert_eq!(calls[1].items.len(), 2);
    assert_eq!(calls[2].sink, SinkUse::Empty);
}

#[tokio::test]
async fn tool_call_flush_block_ends_the_stream_before_the_passthrough() {
    let translation = FakeTranslation::planned(vec![Plan::Fail(modify_response(None))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("a"), tool_chunk()],
    )
    .await;
    let original =
        serde_json::to_string(&GuardrailValue::Array(vec![text_chunk("a"), tool_chunk()])).unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(
        block_texts(&outputs),
        vec![format!("started=false;items=0;texts=;original={original}")]
    );
}

#[tokio::test]
async fn tool_call_without_text_does_not_flush() {
    let translation = FakeTranslation::planned(vec![Plan::Pass, Plan::Pass]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![tool_chunk()],
    )
    .await;
    assert_eq!(outputs, vec![Ok(item(tool_chunk()))]);
    let calls = translation.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, SinkUse::Absent);
    assert_eq!(calls[1].sink, SinkUse::Empty);
}

#[tokio::test]
async fn tool_call_passthrough_strips_text_from_the_same_chunk() {
    let translation = FakeTranslation::planned(vec![
        Plan::Pass,
        Plan::Rewrite {
            index: 0,
            text: "secret",
            holdback: 0,
        },
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![mixed_tool_chunk("secret", None)],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(tool_chunk())),
            Ok(item(synthetic("secret", Some("assistant"), None))),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls[1].items, vec![mixed_tool_chunk("secret", None)]);
}

#[tokio::test]
async fn tool_call_inspection_gets_a_deep_clone() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "hi",
            holdback: 0,
        },
        Plan::Mutate,
        Plan::Pass,
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("hi"), tool_chunk()],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("hi", Some("assistant"), None))),
            Ok(item(tool_chunk())),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[1].sink, SinkUse::Absent);
    assert_eq!(calls[2].items, vec![text_chunk("hi"), tool_chunk()]);
}

#[tokio::test]
async fn tool_call_inspection_waits_for_the_end_of_the_source() {
    let translation = FakeTranslation::planned(vec![
        Plan::Pass,
        Plan::Rewrite {
            index: 0,
            text: "x",
            holdback: 0,
        },
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let source = vec![tool_chunk(), text_chunk("x")];
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        source.clone(),
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(tool_chunk())),
            Ok(item(synthetic("x", Some("assistant"), None))),
        ]
    );
    let calls = translation.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, SinkUse::Absent);
    assert_eq!(calls[0].items, source);
    assert_eq!(calls[1].sink, SinkUse::Empty);
    assert_eq!(calls[1].items, source);
}

#[tokio::test]
async fn tool_call_passthrough_defers_a_finish_reason_to_the_terminator() {
    let translation = FakeTranslation::planned(vec![Plan::Pass, Plan::Pass]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![mixed_tool_chunk("done", Some("stop"))],
    )
    .await;
    assert_eq!(
        outputs,
        vec![Ok(item(tool_chunk())), Ok(item(terminator("stop")))]
    );
}

#[tokio::test]
async fn source_failure_after_a_chunk_is_the_only_output() {
    let translation = FakeTranslation::planned(vec![Plan::Pass]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_results(
        params(&translation, &guardrail, &mut data, &keys),
        vec![Ok(text_chunk("a")), Err(SourceFailure)],
    )
    .await;
    assert_eq!(
        outputs,
        vec![Err(StreamLoopError::Source {
            detail: SourceFailure.to_string(),
        })]
    );
    assert!(translation.calls().is_empty());
}

#[tokio::test]
async fn source_failure_keeps_the_queued_output_before_the_error() {
    let translation = FakeTranslation::planned(vec![Plan::Rewrite {
        index: 0,
        text: "ab",
        holdback: 0,
    }]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_results(loop_params, vec![Ok(text_chunk("a")), Err(SourceFailure)]).await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("ab", Some("assistant"), None))),
            Err(StreamLoopError::Source {
                detail: SourceFailure.to_string(),
            }),
        ]
    );
    assert_eq!(translation.calls().len(), 1);
}
