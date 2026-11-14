#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::unified_dispatch::emit::{HttpErrorDetail, StreamHttpError};
use indexmap::IndexMap;

use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::tests::support::{
    FakeGuardrail, FakeTranslation, Plan, SinkUse, block_texts, collect_outputs, item,
    modify_response, object, params, rewrite, synthetic, text_chunk,
};

#[tokio::test]
async fn round_starts_each_round_with_a_fresh_sink() {
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
        Plan::Pass,
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
            Ok(item(synthetic("a", Some("assistant"), None))),
            Ok(item(synthetic("b", None, None))),
        ]
    );
    let sinks: Vec<SinkUse> = translation.calls().iter().map(|call| call.sink).collect();
    assert_eq!(sinks, vec![SinkUse::Empty, SinkUse::Empty, SinkUse::Empty]);
}

#[tokio::test]
async fn round_yields_synthetics_in_the_order_they_are_sent() {
    let translation = FakeTranslation::planned(vec![
        rewrite(0, "a", 0),
        rewrite(0, "ab", 0),
        rewrite(0, "abc", 0),
        Plan::Fail(GuardrailFailure::Http {
            status: 400,
            detail: "blocked".to_owned(),
        }),
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(
        loop_params,
        vec![text_chunk("a"), text_chunk("b"), text_chunk("c")],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(synthetic("a", Some("assistant"), None))),
            Ok(item(synthetic("b", None, None))),
            Ok(item(synthetic("c", None, None))),
            Ok(item(GuardrailValue::Text(
                "status=400;items=3;texts=a|b|c".to_owned(),
            ))),
        ]
    );
}

#[tokio::test]
async fn block_round_before_any_output_sends_a_standalone_block() {
    let translation = FakeTranslation::planned(vec![Plan::Fail(modify_response(None))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a"), text_chunk("b")]).await;
    let original = serde_json::to_string(&GuardrailValue::Array(vec![text_chunk("a")])).unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(
        block_texts(&outputs),
        vec![format!("started=false;items=0;texts=;original={original}")]
    );
    assert_eq!(translation.calls().len(), 1);
}

#[tokio::test]
async fn block_round_after_output_uses_the_yielded_items() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "a",
            holdback: 0,
        },
        Plan::Fail(modify_response(None)),
    ]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a"), text_chunk("b")]).await;
    let original = serde_json::to_string(&GuardrailValue::Array(vec![
        text_chunk("a"),
        text_chunk("b"),
    ]))
    .unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(
        outputs[0],
        Ok(item(synthetic("a", Some("assistant"), None)))
    );
    assert_eq!(
        block_texts(&outputs),
        vec![format!("started=true;items=1;texts=a;original={original}")]
    );
}

#[tokio::test]
async fn block_round_keeps_a_given_original_response() {
    let original = GuardrailValue::Text("given".to_owned());
    let translation =
        FakeTranslation::planned(vec![Plan::Fail(modify_response(Some(original.clone())))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a")]).await;
    assert_eq!(
        block_texts(&outputs),
        vec![format!(
            "started=false;items=0;texts=;original={}",
            serde_json::to_string(&original).unwrap()
        )]
    );
}

#[tokio::test]
async fn block_round_without_a_terminator_ends_the_stream_with_the_failure() {
    let translation = FakeTranslation::without_terminator(vec![Plan::Fail(modify_response(None))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a")]).await;
    let expected = modify_response(Some(GuardrailValue::Array(vec![text_chunk("a")])));
    assert_eq!(
        outputs,
        vec![Err(StreamLoopError::Block { source: expected })]
    );
}

#[tokio::test]
async fn http_error_round_sends_the_stream_error_items() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "a",
            holdback: 0,
        },
        Plan::Fail(GuardrailFailure::Http {
            status: 400,
            detail: "blocked".to_owned(),
        }),
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
            Ok(item(synthetic("a", Some("assistant"), None))),
            Ok(item(GuardrailValue::Text(
                "status=400;items=1;texts=a".to_owned()
            ))),
        ]
    );
}

#[tokio::test]
async fn http_error_round_before_any_output_leaves_the_stream() {
    let translation = FakeTranslation::planned(vec![Plan::Fail(GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    })]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a")]).await;
    let expected = StreamLoopError::Http {
        source: StreamHttpError {
            status_code: 400,
            detail: HttpErrorDetail::Text("blocked".to_owned()),
        },
    };
    assert_eq!(outputs, vec![Err(expected)]);
}

#[tokio::test]
async fn http_error_round_with_object_detail_sends_the_stream_error_items() {
    let mut detail = IndexMap::new();
    detail.insert(
        "error".to_owned(),
        GuardrailValue::Text("denied".to_owned()),
    );
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "a",
            holdback: 0,
        },
        Plan::Fail(GuardrailFailure::HttpObject {
            status: 403,
            detail,
        }),
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
            Ok(item(synthetic("a", Some("assistant"), None))),
            Ok(item(GuardrailValue::Text(
                "status=403;items=1;texts=a".to_owned()
            ))),
        ]
    );
}

#[tokio::test]
async fn http_error_underflow_in_a_round_sends_the_transform_error() {
    let translation = FakeTranslation::planned(vec![
        Plan::Rewrite {
            index: 0,
            text: "abc",
            holdback: 0,
        },
        Plan::Rewrite {
            index: 0,
            text: "xyz",
            holdback: 0,
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
            Ok(item(GuardrailValue::Text(
                "status=400;items=1;texts=abc".to_owned()
            ))),
        ]
    );
}

#[tokio::test]
async fn round_reads_the_logging_target_from_request_data() {
    let mut data = BTreeMap::new();
    data.insert(
        "litellm_logging_obj".to_owned(),
        object(vec![
            (
                "litellm_params",
                object(vec![("a", GuardrailValue::SignedInteger(1))]),
            ),
            (
                "model_call_details",
                object(vec![(
                    "litellm_params",
                    object(vec![("b", GuardrailValue::SignedInteger(2))]),
                )]),
            ),
        ]),
    );
    let translation = FakeTranslation::planned(vec![Plan::Pass]);
    let guardrail = FakeGuardrail;
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("a")],
    )
    .await;
    assert!(outputs.is_empty());
    let calls = translation.calls();
    assert_eq!(calls.len(), 1);
    let target = GuardrailLogTarget {
        litellm_params: Some(BTreeMap::from([(
            "a".to_owned(),
            GuardrailValue::SignedInteger(1),
        )])),
        model_call_params: Some(BTreeMap::from([(
            "b".to_owned(),
            GuardrailValue::SignedInteger(2),
        )])),
    };
    assert_eq!(calls[0].log_target, Some(target));
}

#[tokio::test]
async fn other_round_failure_leaves_the_stream() {
    let translation =
        FakeTranslation::planned(vec![Plan::Fail(GuardrailFailure::Other("boom".to_owned()))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let mut loop_params = params(&translation, &guardrail, &mut data, &keys);
    loop_params.sampling_rate = 1;
    let outputs = collect_outputs(loop_params, vec![text_chunk("a")]).await;
    assert_eq!(
        outputs,
        vec![Err(StreamLoopError::Guardrail {
            source: GuardrailFailure::Other("boom".to_owned()),
        })]
    );
}

#[tokio::test]
async fn round_without_a_logging_target_passes_none() {
    let translation = FakeTranslation::planned(vec![Plan::Pass]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![text_chunk("a")],
    )
    .await;
    assert!(outputs.is_empty());
    let calls = translation.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].log_target, None);
}
