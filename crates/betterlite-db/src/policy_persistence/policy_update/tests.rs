#![cfg(test)]

use betterlite_policy::records::crud::PolicyUpdateRequest;
use rstest::rstest;

use super::update_policy_in_db;
use crate::policy_persistence::tests::{Call, Fixture, NOW, id_filter, policy_row};

const CHANGES: &str = r#"{"policy_name": "pii-v2", "inherit": "base", "description": "tightened", "guardrails_add": ["presidio"], "guardrails_remove": ["legacy"], "condition": {"model": "gpt-4o"}, "pipeline": {"mode": "pre_call", "steps": [{"guardrail": "presidio", "on_fail": "next"}]}}"#;

const PIPELINE: &str = r#""pipeline":{"mode":"pre_call","steps":[{"guardrail":"presidio","modify_response_message":null,"on_error":null,"on_fail":"next","on_pass":"allow","pass_data":false}]}"#;

#[rstest]
#[case::named_editor(Some("editor"), r#""editor""#)]
#[case::anonymous(None, "null")]
#[tokio::test]
async fn updates_a_draft_with_only_the_fields_it_sets(
    #[case] updated_by: Option<&str>,
    #[case] written_by: &str,
) {
    let fixture = Fixture::new(
        &[policy_row("p1", "pii", "draft", "2026-01-01T00:00:00Z")],
        &[],
    );
    let request: PolicyUpdateRequest = serde_json::from_str(CHANGES).unwrap();
    let response = update_policy_in_db(&fixture.policy_store(), "p1", &request, updated_by)
        .await
        .unwrap();
    let calls = fixture.client.policies.calls();
    let [Call::FindUnique(lookup), Call::Update(filter, data)] = calls.as_slice() else {
        panic!("expected a lookup and an update, got {calls:?}");
    };
    assert_eq!(
        (lookup, filter),
        (&id_filter("policy_id", "p1"), &id_filter("policy_id", "p1"))
    );
    let written = format!(
        r#"{{"condition":{{"model":"gpt-4o"}},"description":"tightened","guardrails_add":["presidio"],"guardrails_remove":["legacy"],"inherit":"base",{PIPELINE},"policy_name":"pii-v2","updated_at":"{NOW}","updated_by":{written_by}}}"#
    );
    assert_eq!(serde_json::to_string(data).unwrap(), written);
    let returned = format!(
        r#"{{"policy_id":"p1","policy_name":"pii-v2","version_number":1,"version_status":"draft","parent_version_id":null,"is_latest":true,"published_at":null,"production_at":null,"inherit":"base","description":"tightened","guardrails_add":["presidio"],"guardrails_remove":["legacy"],"condition":{{"model":"gpt-4o"}},{PIPELINE},"created_at":"2026-01-01T00:00:00Z","updated_at":"{NOW}","created_by":null,"updated_by":{written_by},"definition_location":"db"}}"#
    );
    assert_eq!(serde_json::to_string(&response).unwrap(), returned);
    assert!(fixture.policies.into_inner().unwrap().policies.is_empty());
}

#[rstest]
#[case::named_editor(Some("editor"), r#""editor""#)]
#[case::anonymous(None, "null")]
#[tokio::test]
async fn writes_only_the_fields_the_request_sets(
    #[case] updated_by: Option<&str>,
    #[case] written_by: &str,
) {
    let fixture = Fixture::new(
        &[policy_row("p1", "pii", "draft", "2026-01-01T00:00:00Z")],
        &[],
    );
    let request: PolicyUpdateRequest =
        serde_json::from_str(r#"{"description": "tightened"}"#).unwrap();
    update_policy_in_db(&fixture.policy_store(), "p1", &request, updated_by)
        .await
        .unwrap();
    let calls = fixture.client.policies.calls();
    let [Call::FindUnique(_), Call::Update(_, data)] = calls.as_slice() else {
        panic!("expected a lookup and an update, got {calls:?}");
    };
    let written =
        format!(r#"{{"description":"tightened","updated_at":"{NOW}","updated_by":{written_by}}}"#);
    assert_eq!(serde_json::to_string(data).unwrap(), written);
}

#[rstest]
#[case::production(Some("production"), "production")]
#[case::published(Some("published"), "published")]
#[case::status_missing(None, "production")]
#[tokio::test]
async fn refuses_every_version_but_a_draft(#[case] stored: Option<&str>, #[case] reported: &str) {
    let mut row = policy_row("p1", "pii", "draft", "2026-01-01T00:00:00Z");
    row.version_status = stored.map(str::to_owned);
    let fixture = Fixture::new(&[row], &[]);
    let request: PolicyUpdateRequest = serde_json::from_str(r#"{"description": "x"}"#).unwrap();
    let error = update_policy_in_db(&fixture.policy_store(), "p1", &request, None)
        .await
        .unwrap_err();
    let expected = format!(
        "Error updating policy in DB: Only draft versions can be updated. This policy has status '{reported}'."
    );
    assert_eq!(error.to_string(), expected);
    assert_eq!(fixture.client.policies.calls().len(), 1);
}

#[rstest]
#[case::unknown_id(
    "nope",
    None,
    "Error updating policy in DB: Policy with ID nope not found"
)]
#[case::failed_write(
    "p1",
    Some("update"),
    "Error updating policy in DB: table refused update"
)]
#[tokio::test]
async fn reports_lookup_and_write_failures(
    #[case] policy_id: &str,
    #[case] failing: Option<&'static str>,
    #[case] message: &str,
) {
    let fixture = Fixture::new(
        &[policy_row("p1", "pii", "draft", "2026-01-01T00:00:00Z")],
        &[],
    );
    if let Some(method) = failing {
        fixture.client.policies.fail(method);
    }
    let request: PolicyUpdateRequest = serde_json::from_str("{}").unwrap();
    let error = update_policy_in_db(&fixture.policy_store(), policy_id, &request, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
}
