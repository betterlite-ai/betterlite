#![cfg(test)]

use crate::analysis::{Candidate, Examined, ModelCall, Observation, ObservationKind, ReadContent};
use crate::models::execution::{Execution, ExecutionContent, ExecutionSource, TracePart};
use crate::models::messages::{Claim, ModelResult};
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};

pub(super) fn claim() -> Claim {
    serde_json::from_str(include_str!("fixtures/claim.json")).unwrap()
}

pub(super) fn execution(id: &str) -> Execution {
    Execution {
        id: id.to_owned(),
        source: ExecutionSource::Traces,
        trace_id: String::from("trace"),
        trace_ref: String::new(),
        team_id: String::from("team"),
        name: String::from("name"),
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
        name: String::from("span"),
        kind: String::from("llm"),
        content: content.to_owned(),
        truncated: false,
    }
}

pub(super) fn examined(execution_id: &str, content: &str) -> Examined {
    let observation = Observation {
        check_id: String::from("check"),
        kind: ObservationKind::Issue,
        summary: String::from("a supporting observation"),
        evidence: Vec::new(),
    };
    let trace_part = part(execution_id, "s1", content);
    let mut record = Examined {
        execution: execution(execution_id),
        observations: Vec::new(),
        parts: Vec::new(),
        partial: false,
        cannot_assess: false,
    };
    record.observations.push(observation);
    record.parts.push(trace_part);
    record
}

pub(super) fn candidate(execution_ids: Vec<String>) -> Candidate {
    Candidate {
        check_id: String::from("check"),
        kind: ObservationKind::Issue,
        title: String::from("title"),
        hypothesis: String::from("hypothesis"),
        execution_ids,
        existing_finding_id: None,
    }
}

pub(super) fn never_read() -> ReadContent {
    Arc::new(|_, _, _| Box::pin(async { panic!("read should not be called in this test") }))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ReadCall {
    pub execution_id: String,
    pub cursor: String,
    pub offset: i64,
}

type Recorder<T> = Arc<Mutex<Vec<T>>>;

fn recorder<T>() -> (Recorder<T>, Recorder<T>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::clone(&log);
    (log, handle)
}

pub(super) fn sequential_read(pages: Vec<ExecutionContent>) -> (ReadContent, Recorder<ReadCall>) {
    let (calls, calls_handle) = recorder();
    let remaining = Arc::new(Mutex::new(pages));
    let read: ReadContent = Arc::new(move |execution_id, cursor, offset| {
        calls_handle.lock().unwrap().push(ReadCall {
            execution_id,
            cursor,
            offset,
        });
        let page = remaining.lock().unwrap().remove(0);
        Box::pin(async move { Ok(page) })
    });
    (read, calls)
}

pub(super) fn scripted(responses: Vec<String>) -> (ModelCall, Recorder<String>) {
    let (prompts, prompts_handle) = recorder();
    let model: ModelCall = Arc::new(move |request| {
        let task_prompts = Arc::clone(&prompts_handle);
        let scripted_responses = responses.clone();
        Box::pin(async move {
            let mut sent = task_prompts.lock().unwrap();
            let index = sent.len();
            sent.push(request.prompt);
            drop(sent);
            let content = scripted_responses
                .get(index)
                .or_else(|| scripted_responses.last())
                .cloned()
                .unwrap_or_default();
            Ok(ModelResult {
                content,
                cost: Decimal::ZERO,
            })
        })
    });
    (model, prompts)
}

pub(super) fn submit_decision(execution_id: &str, span_id: &str, quote: &str) -> String {
    format!(
        r#"{{"action":"submit","finding":{{"title":"Observed issue","description":"Found a concrete issue in the trace.","check_id":"check","evidence":[{{"execution_id":"{execution_id}","span_id":"{span_id}","quote":"{quote}"}}]}}}}"#
    )
}

pub(super) fn inconclusive_decision() -> String {
    String::from(r#"{"action":"inconclusive"}"#)
}

pub(super) fn observations_decision(page: i64) -> String {
    format!(r#"{{"action":"observations","page":{page}}}"#)
}

pub(super) fn catalog_decision(page: i64) -> String {
    format!(r#"{{"action":"catalog","page":{page}}}"#)
}

pub(super) fn feedback_decision(page: i64) -> String {
    format!(r#"{{"action":"feedback","page":{page}}}"#)
}

pub(super) fn read_decision(execution_id: &str, cursor: &str, offset: i64) -> String {
    format!(
        r#"{{"action":"read","execution_id":"{execution_id}","cursor":"{cursor}","offset":{offset}}}"#
    )
}
