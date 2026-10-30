#![cfg(test)]

use super::fixtures;
use crate::analysis::DecisionAction;
use crate::investigation::investigation_decision;
use crate::models::messages::{ModelRequest, Purpose};

fn request() -> ModelRequest {
    ModelRequest {
        prompt: String::from("investigate"),
        purpose: Purpose::Investigate,
    }
}

#[tokio::test]
async fn decision_schema_uses_decision_for_multi_step_requests() {
    let (model, _) = fixtures::scripted(vec![String::from(
        r#"{"action":"read","execution_id":"e1","cursor":"c","offset":3}"#,
    )]);
    let decision = investigation_decision(request(), &model, 2).await.unwrap();
    assert_eq!(decision.action, DecisionAction::Read);
    assert_eq!(decision.execution_id, Some(String::from("e1")));
    assert_eq!(decision.cursor, "c");
    assert_eq!(decision.offset, 3);
}

#[tokio::test]
async fn decision_schema_converts_final_decision_to_decision_on_the_last_step() {
    let (model, _) = fixtures::scripted(vec![fixtures::submit_decision(
        "e1",
        "s1",
        "the original quote",
    )]);
    let decision = investigation_decision(request(), &model, 1).await.unwrap();
    assert_eq!(decision.action, DecisionAction::Submit);
    assert_eq!(decision.page, 0);
    assert_eq!(decision.execution_id, None);
    assert_eq!(decision.cursor, String::new());
    assert_eq!(decision.offset, 0);
    assert!(decision.finding.is_some());
}

#[tokio::test]
async fn decision_schema_rejects_navigation_fields_on_the_final_step() {
    let (model, _) = fixtures::scripted(vec![String::from(
        r#"{"action":"read","execution_id":"e1"}"#,
    )]);
    investigation_decision(request(), &model, 1)
        .await
        .unwrap_err();
}
