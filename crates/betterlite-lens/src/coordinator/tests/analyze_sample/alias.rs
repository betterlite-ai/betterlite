use super::super::fixtures::{
    capture_progress, claim, execution, extraction_observation, inconclusive_decision,
    no_extraction_observation, page, part, reader, submit_decision,
};
use crate::analysis::{ModelCall, ReadContent};
use crate::models::execution::Sample;
use crate::models::messages::{ModelResult, Purpose};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn two_execution_setup() -> (Sample, ReadContent) {
    let sample = Sample {
        executions: vec![execution("exec-a"), execution("exec-b")],
        eligible: 2,
        selected: 2,
        next_offset: None,
        next_cursor: None,
    };
    let read = reader(vec![
        page(part("exec-a", "s0", "first content")),
        page(part("exec-b", "s1", "second content")),
    ]);
    (sample, read)
}

fn capturing_model(responses: Vec<String>) -> (ModelCall, Arc<Mutex<Vec<String>>>) {
    let pending = Mutex::new(VecDeque::from(responses));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::clone(&prompts);
    let model: ModelCall = Arc::new(move |request| {
        handle.lock().unwrap().push(request.prompt.clone());
        let content = match request.purpose {
            Purpose::Cluster => super::super::fixtures::echo_candidates(&request.prompt),
            Purpose::Extract | Purpose::Investigate => pending.lock().unwrap().pop_front().unwrap(),
        };
        Box::pin(async move {
            Ok(ModelResult {
                content,
                cost: rust_decimal::Decimal::ZERO,
            })
        })
    });
    (model, prompts)
}

#[tokio::test]
async fn maps_executions_to_run_aliases_and_restores_original_ids() {
    let mut input = claim();
    input.job.settings.concurrency = 1;
    let (progress, _log) = capture_progress();
    let (sample, read) = two_execution_setup();
    let responses = vec![
        extraction_observation("r0", "s0", "expected_behavior", "first content"),
        no_extraction_observation(),
        submit_decision("r0", "s0", "expected_behavior", "first content"),
    ];
    let (model, prompts) = capturing_model(responses);
    let result = super::super::analyze_sample(input, sample, read, model, progress)
        .await
        .unwrap();
    let sent: Vec<String> = prompts.lock().unwrap().clone();
    assert!(sent.iter().any(|prompt| prompt.contains("\"r0\"")));
    assert!(
        sent.iter()
            .all(|prompt| !prompt.contains("exec-a") && !prompt.contains("exec-b"))
    );
    let mut assessed_ids: Vec<_> = result
        .assessments
        .iter()
        .map(|item| item.execution_id.clone())
        .collect();
    assessed_ids.sort();
    assert_eq!(
        assessed_ids,
        vec![String::from("exec-a"), String::from("exec-b")]
    );
    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].evidence[0].execution_id, "exec-a");
}

#[tokio::test]
async fn restores_original_ids_when_the_result_has_no_findings() {
    let mut input = claim();
    input.job.settings.concurrency = 1;
    let (progress, _log) = capture_progress();
    let sample = Sample {
        executions: vec![execution("exec-a")],
        eligible: 1,
        selected: 1,
        next_offset: None,
        next_cursor: None,
    };
    let read = reader(vec![page(part("exec-a", "s0", "only content"))]);
    let responses = vec![
        extraction_observation("r0", "s0", "expected_behavior", "only content"),
        inconclusive_decision(),
    ];
    let (model, _prompts) = capturing_model(responses);
    let result = super::super::analyze_sample(input, sample, read, model, progress)
        .await
        .unwrap();
    assert_eq!(result.assessments.len(), 1);
    assert_eq!(result.assessments[0].execution_id, "exec-a");
    assert!(result.findings.is_empty());
    assert_eq!(result.coverage.investigated, 1);
}
