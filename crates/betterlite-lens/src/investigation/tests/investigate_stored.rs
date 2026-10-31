#![cfg(test)]

use super::fixtures;
use crate::analysis::{Examined, Observation, ObservationKind};
use crate::investigation::loop_state::{self, Context};
use crate::models::settings::Check;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ReadCount {
    completed_read_count: i64,
}

#[derive(Deserialize)]
struct EvidencePrompt {
    evidence: Vec<EvidencePart>,
}

#[derive(Deserialize)]
struct EvidencePart {
    span_id: String,
}

#[derive(Deserialize, Serialize)]
struct QuestionsPrompt {
    questions: Vec<Check>,
}

#[tokio::test]
async fn full_loop_pages_through_navigation_before_submitting() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![
        fixtures::observations_decision(0),
        fixtures::catalog_decision(0),
        fixtures::feedback_decision(0),
        fixtures::submit_decision("e1", "s1", "the original quote"),
    ]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_some());

    assert_eq!(prompts.lock().unwrap().len(), 4);
    let read_counts: Vec<i64> = prompts
        .lock()
        .unwrap()
        .iter()
        .map(|prompt| {
            let decoded: ReadCount = serde_json::from_str(prompt).unwrap();
            decoded.completed_read_count
        })
        .collect();
    assert_eq!(read_counts, vec![0, 1, 2, 3]);
}

#[tokio::test]
async fn catalog_partitioning_sizes_only_the_execution_not_the_trace_content() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let large_content = format!("the original quote{}", "x".repeat(20_000));
    let examined = vec![fixtures::examined("e1", &large_content)];
    let read = fixtures::never_read();
    let (model, _) = fixtures::scripted(vec![fixtures::submit_decision(
        "e1",
        "s1",
        "the original quote",
    )]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_some());
}

#[tokio::test]
async fn an_execution_can_contribute_more_than_one_matching_observation() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let observation = Observation {
        check_id: String::from("check"),
        kind: ObservationKind::Issue,
        summary: String::from("a supporting observation"),
        evidence: Vec::new(),
    };
    let examined = vec![Examined {
        execution: fixtures::execution("e1"),
        observations: vec![observation.clone(), observation],
        parts: vec![fixtures::part("e1", "s1", "the original quote")],
        partial: false,
        cannot_assess: false,
    }];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn non_ascii_observations_are_sized_by_serialized_characters() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let observation = Observation {
        check_id: String::from("check"),
        kind: ObservationKind::Issue,
        summary: "\u{e9}".repeat(8_451),
        evidence: Vec::new(),
    };
    let examined = vec![Examined {
        execution: fixtures::execution("e1"),
        observations: vec![observation],
        parts: vec![fixtures::part("e1", "s1", "the original quote")],
        partial: false,
        cannot_assess: false,
    }];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);
    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };

    let result = loop_state::run(context).await.unwrap();

    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn equal_priority_evidence_keeps_insertion_order_across_page_boundary() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![Examined {
        execution: fixtures::execution("e1"),
        observations: Vec::new(),
        parts: vec![
            fixtures::part("e1", "z", &"z".repeat(29_000)),
            fixtures::part("e1", "a", &"a".repeat(1_000)),
        ],
        partial: false,
        cannot_assess: false,
    }];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);
    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };

    let result = loop_state::run(context).await.unwrap();

    assert!(result.finding.is_none());
    let prompt_text = prompts.lock().unwrap().first().cloned().unwrap();
    let evidence_prompt: EvidencePrompt = serde_json::from_str(&prompt_text).unwrap();
    assert_eq!(evidence_prompt.evidence.len(), 1);
    assert_eq!(evidence_prompt.evidence.first().unwrap().span_id, "z");
}

#[tokio::test]
async fn investigation_prompt_serializes_analysis_checks_as_questions() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);
    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };

    loop_state::run(context).await.unwrap();

    let prompt_text = prompts.lock().unwrap().first().cloned().unwrap();
    let questions_prompt: QuestionsPrompt = serde_json::from_str(&prompt_text).unwrap();
    assert_eq!(questions_prompt.questions.len(), 1);
}

#[tokio::test]
async fn an_empty_candidate_title_still_builds_a_prompt() {
    let claim = fixtures::claim();
    let mut candidate = fixtures::candidate(vec![String::from("e1")]);
    candidate.title = String::new();
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mismatched_execution_ids_still_run_the_loop() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("missing")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}
