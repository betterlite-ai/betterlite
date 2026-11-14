#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::tests::support::{
    FakeGuardrail, FakeTranslation, Plan, block_texts, collect_outputs, item, modify_response,
    params, tool_chunk,
};

#[tokio::test]
async fn block_inspection_sends_the_block_after_the_yielded_items() {
    let translation = FakeTranslation::planned(vec![Plan::Fail(modify_response(None))]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![tool_chunk()],
    )
    .await;
    let original = serde_json::to_string(&GuardrailValue::Array(vec![tool_chunk()])).unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0], Ok(item(tool_chunk())));
    assert_eq!(
        block_texts(&outputs),
        vec![format!("started=true;items=1;texts=;original={original}")]
    );
    assert_eq!(translation.calls().len(), 1);
}

#[tokio::test]
async fn http_error_inspection_leaves_the_stream() {
    let translation = FakeTranslation::planned(vec![Plan::Fail(GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    })]);
    let guardrail = FakeGuardrail;
    let mut data = BTreeMap::new();
    let keys = BTreeMap::new();
    let outputs = collect_outputs(
        params(&translation, &guardrail, &mut data, &keys),
        vec![tool_chunk()],
    )
    .await;
    assert_eq!(
        outputs,
        vec![
            Ok(item(tool_chunk())),
            Err(StreamLoopError::Guardrail {
                source: GuardrailFailure::Http {
                    status: 400,
                    detail: "blocked".to_owned(),
                },
            }),
        ]
    );
    assert_eq!(translation.calls().len(), 1);
}
