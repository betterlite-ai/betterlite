#![cfg(test)]
use super::fixtures::candidate;
use super::merge::merge_candidates;
use crate::analysis::{ModelCall, ObservationKind};
use crate::models::messages::ModelResult;
use std::sync::{Arc, Mutex};

fn recording_model(response: String) -> (ModelCall, Arc<Mutex<Vec<String>>>) {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::clone(&prompts);
    let model: ModelCall = Arc::new(move |request| {
        handle.lock().unwrap().push(request.prompt);
        let content = response.clone();
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
async fn sends_p_aliased_identities_in_the_prompt() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"a","hypothesis":"a","execution_ids":["p0"]},{"check_id":"b","kind":"issue","title":"b","hypothesis":"b","execution_ids":["p1"]}]}"#;
    let (model, prompts) = recording_model(response.to_owned());
    let candidates = vec![
        candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]),
        candidate("b", ObservationKind::Issue, "b", "b", vec!["r1"]),
    ];
    merge_candidates(candidates, 0, &model).await.unwrap();
    let sent: Vec<String> = prompts.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("\"p0\""));
    assert!(sent[0].contains("\"p1\""));
    assert!(sent[0].contains("response_schema"));
}

#[tokio::test]
async fn expands_identity_references_back_to_original_execution_ids() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"merged","hypothesis":"merged","execution_ids":["p0","p1"]}]}"#;
    let (model, _prompts) = recording_model(response.to_owned());
    let candidates = vec![
        candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]),
        candidate("a", ObservationKind::Issue, "a", "a", vec!["r1"]),
    ];
    let (active, settled) = merge_candidates(candidates, 0, &model).await.unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(
        active[0].execution_ids,
        vec![String::from("r0"), String::from("r1")]
    );
}
