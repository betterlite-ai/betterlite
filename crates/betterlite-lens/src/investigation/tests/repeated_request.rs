#![cfg(test)]

use super::fixtures;
use crate::investigation::loop_state::{self, Context};
use serde::Deserialize;

#[derive(Deserialize)]
struct MustDecide {
    must_decide: bool,
}

#[tokio::test]
async fn detection_forces_a_final_decision_after_a_repeated_navigation_request() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![
        fixtures::observations_decision(0),
        fixtures::observations_decision(0),
        fixtures::inconclusive_decision(),
    ]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());

    assert_eq!(prompts.lock().unwrap().len(), 3);
    let decoded: MustDecide = serde_json::from_str(&prompts.lock().unwrap()[2]).unwrap();
    assert!(decoded.must_decide);
}
