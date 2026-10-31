#![cfg(test)]

use super::fixtures;
use crate::analysis::DecisionAction;
use crate::investigation::{investigate, investigation_decision};
use crate::models::messages::{ModelRequest, Purpose};
use std::collections::BTreeMap;

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

#[derive(serde::Deserialize)]
struct ProbeSchema {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(rename = "additionalProperties", default)]
    additional_properties: Option<bool>,
    #[serde(default)]
    properties: BTreeMap<String, Self>,
    #[serde(rename = "anyOf", default)]
    any_of: Vec<Self>,
    #[serde(default)]
    items: Option<Box<Self>>,
}

#[derive(serde::Deserialize)]
struct ProbePrompt {
    response_schema: ProbeSchema,
}

#[tokio::test]
async fn decision_schema_forbids_additional_properties_throughout() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "nothing to see here")];
    let (model, prompts) = fixtures::scripted(vec![fixtures::submit_decision(
        "e1",
        "s1",
        "the original quote",
    )]);
    let read = fixtures::never_read();

    investigate(claim, candidate, examined, read, model)
        .await
        .unwrap();

    let sent = prompts.lock().unwrap();
    let probe: ProbePrompt = serde_json::from_str(&sent[0]).unwrap();
    drop(sent);
    let schema = probe.response_schema;
    assert_eq!(schema.additional_properties, Some(false));
    let finding_object = schema.properties["finding"]
        .any_of
        .iter()
        .find(|branch| branch.kind.as_deref() == Some("object"))
        .unwrap();
    assert_eq!(finding_object.additional_properties, Some(false));
    let evidence_items = finding_object.properties["evidence"]
        .items
        .as_ref()
        .unwrap();
    assert_eq!(evidence_items.additional_properties, Some(false));
}
