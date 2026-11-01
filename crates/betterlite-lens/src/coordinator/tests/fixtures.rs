#![cfg(test)]

use crate::analysis::{
    Candidate, Clusters, ModelCall, ModelFuture, Observation, ObservationKind, ReadContent,
    ReportProgress,
};
use crate::models::execution::{Coverage, Execution, ExecutionContent, ExecutionSource, TracePart};
use crate::models::finding::{Evidence, Role};
use crate::models::messages::{Claim, ModelResult};
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub(super) type ProgressLog = Arc<Mutex<Vec<(String, Coverage)>>>;

#[derive(Deserialize)]
struct SentCandidates {
    candidates: Vec<Candidate>,
}

pub(super) fn echo_candidates(prompt: &str) -> String {
    let sent: SentCandidates = serde_json::from_str(prompt).unwrap();
    serde_json::to_string(&Clusters {
        candidates: sent.candidates,
    })
    .unwrap()
}

pub(super) fn claim() -> Claim {
    let mut claim: Claim =
        serde_json::from_str(include_str!("../../analysis/fixtures/claim.json")).unwrap();
    claim.findings.clear();
    claim
}

pub(super) fn execution(id: &str) -> Execution {
    Execution {
        id: id.to_owned(),
        source: ExecutionSource::Traces,
        trace_id: String::from("trace"),
        trace_ref: String::new(),
        team_id: String::from("team"),
        name: String::from("run"),
        start_time: String::from("2026-01-01T00:00:00Z"),
        span_count: 1,
        root_seen: false,
        service: String::new(),
        metadata: Vec::new(),
    }
}

pub(super) fn part(execution_id: &str, span_id: &str, content: &str) -> TracePart {
    TracePart {
        execution_id: execution_id.to_owned(),
        span_id: span_id.to_owned(),
        parent_span_id: String::new(),
        name: String::from("root"),
        kind: String::from("agent"),
        content: content.to_owned(),
        truncated: false,
    }
}

pub(super) fn page(part: TracePart) -> ExecutionContent {
    let execution_id = part.execution_id.clone();
    ExecutionContent {
        execution: execution(&execution_id),
        parts: vec![part],
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

pub(super) fn never_read() -> ReadContent {
    Arc::new(|_, _, _| Box::pin(async { panic!("read should not be called in this test") }))
}

pub(super) fn model(responses: Vec<String>) -> ModelCall {
    let pending = Mutex::new(VecDeque::from(responses));
    Arc::new(move |_| model_response(pending.lock().unwrap().pop_front().unwrap()))
}

fn model_response(content: String) -> ModelFuture {
    Box::pin(async move {
        Ok(ModelResult {
            content,
            cost: rust_decimal::Decimal::ZERO,
        })
    })
}

pub(super) fn never_model() -> ModelCall {
    Arc::new(|_| Box::pin(async { panic!("model should not be called in this test") }))
}

// Echoes the sent candidates back as the Clusters response: a valid no-op merge.
pub(super) fn echo_model() -> (ModelCall, Arc<Mutex<usize>>) {
    let calls = Arc::new(Mutex::new(0_usize));
    let handle = Arc::clone(&calls);
    let model: ModelCall = Arc::new(move |request| {
        let mut guard = handle.lock().unwrap();
        *guard = guard.saturating_add(1);
        drop(guard);
        Box::pin(async move {
            Ok(ModelResult {
                content: echo_candidates(&request.prompt),
                cost: rust_decimal::Decimal::ZERO,
            })
        })
    });
    (model, calls)
}

pub(super) fn pipeline_model(
    extraction_responses: Vec<String>,
    investigation_responses: Vec<String>,
) -> ModelCall {
    use crate::models::messages::Purpose;
    let extraction_queue = Mutex::new(VecDeque::from(extraction_responses));
    let investigation_queue = Mutex::new(VecDeque::from(investigation_responses));
    Arc::new(move |request| {
        let content = match request.purpose {
            Purpose::Cluster => echo_candidates(&request.prompt),
            Purpose::Extract => extraction_queue.lock().unwrap().pop_front().unwrap(),
            Purpose::Investigate => investigation_queue.lock().unwrap().pop_front().unwrap(),
        };
        model_response(content)
    })
}

pub(super) fn capture_progress() -> (ReportProgress, ProgressLog) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::clone(&log);
    let progress: ReportProgress = Arc::new(move |stage, coverage| {
        handle.lock().unwrap().push((stage, coverage));
        Box::pin(async { Ok(()) })
    });
    (progress, log)
}

pub(super) fn no_extraction_observation() -> String {
    String::from("{}")
}

fn evidence_json(execution_id: &str, span_id: &str, quote: &str) -> String {
    format!(r#"[{{"execution_id":"{execution_id}","span_id":"{span_id}","quote":"{quote}"}}]"#)
}

pub(super) fn extraction_observation(
    execution_id: &str,
    span_id: &str,
    check_id: &str,
    content: &str,
) -> String {
    format!(
        r#"{{"observations":[{{"check_id":"{check_id}","summary":"supported","evidence":{}}}]}}"#,
        evidence_json(execution_id, span_id, content)
    )
}

pub(super) fn inconclusive_decision() -> String {
    String::from(r#"{"action":"inconclusive"}"#)
}

pub(super) fn submit_decision(
    execution_id: &str,
    span_id: &str,
    check_id: &str,
    quote: &str,
) -> String {
    let mut body = String::from(
        r#"{"action":"submit","finding":{"title":"Observed issue","description":"Found a concrete issue in the trace.","check_id":""#,
    );
    body.push_str(check_id);
    body.push_str(r#"","evidence":"#);
    body.push_str(&evidence_json(execution_id, span_id, quote));
    body.push_str("}}");
    body
}

pub(super) fn candidate(
    check_id: &str,
    kind: ObservationKind,
    title: &str,
    hypothesis: &str,
    execution_ids: Vec<&str>,
) -> Candidate {
    let mut owned_ids = Vec::with_capacity(execution_ids.len());
    for execution_id in execution_ids {
        owned_ids.push(execution_id.to_owned());
    }
    Candidate {
        existing_finding_id: None,
        execution_ids: owned_ids,
        check_id: check_id.to_owned(),
        kind,
        title: title.to_owned(),
        hypothesis: hypothesis.to_owned(),
    }
}

pub(super) fn observation(
    check_id: &str,
    kind: ObservationKind,
    summary: &str,
    execution_ids: Vec<&str>,
) -> Observation {
    let evidence = execution_ids.into_iter().map(supporting_evidence).collect();
    Observation {
        kind,
        evidence,
        check_id: check_id.to_owned(),
        summary: summary.to_owned(),
    }
}

fn supporting_evidence(execution_id: &str) -> Evidence {
    Evidence {
        execution_id: execution_id.to_owned(),
        span_id: String::from("s"),
        quote: String::from("quote"),
        role: Role::Support,
    }
}
