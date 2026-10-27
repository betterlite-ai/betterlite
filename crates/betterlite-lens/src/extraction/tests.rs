#![cfg(test)]

use crate::analysis::{ModelCall, ReadContent};
use crate::models::execution::{Execution, ExecutionContent, TracePart};
use crate::models::messages::{Claim, ModelResult};
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub(super) fn claim() -> Claim {
    let mut claim: Claim =
        serde_json::from_str(include_str!("../analysis/fixtures/claim.json")).unwrap();
    claim.findings.clear();
    claim
}

pub(super) fn execution() -> Execution {
    serde_json::from_str(r#"{"id":"e","source":"traces","trace_id":"t","team_id":"","name":"run","start_time":"","span_count":1}"#).unwrap()
}

pub(super) fn part() -> TracePart {
    serde_json::from_str(
        r#"{"execution_id":"e","span_id":"s","name":"root","kind":"agent","content":"answer"}"#,
    )
    .unwrap()
}

pub(super) fn page() -> ExecutionContent {
    ExecutionContent {
        execution: execution(),
        parts: vec![part()],
        partial: false,
        next_cursor: None,
    }
}

pub(super) fn reader(pages: Vec<ExecutionContent>) -> ReadContent {
    let pending = Mutex::new(VecDeque::from(pages));
    Arc::new(move |_, _, _| {
        let result = pending.lock().unwrap().pop_front().unwrap();
        Box::pin(async move { Ok(result) })
    })
}

pub(super) fn model(responses: Vec<&str>) -> ModelCall {
    let pending = Mutex::new(
        responses
            .into_iter()
            .map(String::from)
            .collect::<VecDeque<_>>(),
    );
    Arc::new(move |_| {
        let content = pending.lock().unwrap().pop_front().unwrap();
        Box::pin(async move {
            Ok(ModelResult {
                content,
                cost: Decimal::ZERO,
            })
        })
    })
}

pub(super) const OBSERVATION: &str = r#"{"observations":[{"check_id":"expected_behavior","summary":"supported","evidence":[{"execution_id":"e","span_id":"s","quote":"answer"}]}]}"#;
