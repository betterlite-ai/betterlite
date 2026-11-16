#![cfg(test)]

use betterlite_policy::records::crud::PolicyCreateRequest;
use betterlite_policy::registry::PolicySource;
use rstest::rstest;
use tracing::Level;

use super::add_policy_to_db;
use crate::policy_persistence::tests::{CapturedLogs, Fixture, NOW};

const FULL_REQUEST: &str = r#"{"policy_name": "pii", "inherit": "base", "description": "masks pii", "guardrails_add": ["presidio"], "guardrails_remove": ["pii-legacy"], "condition": {"model": "gpt-4"}, "pipeline": {"mode": "post_call", "steps": [{"guardrail": "presidio"}]}}"#;

const STORED_FULL: &str = r#"{"condition":{"model":"gpt-4"},"created_at":"NOW","created_by":"admin","description":"masks pii","guardrails_add":["presidio"],"guardrails_remove":["pii-legacy"],"inherit":"base","is_latest":true,"pipeline":{"mode":"post_call","steps":[{"guardrail":"presidio","modify_response_message":null,"on_error":null,"on_fail":"block","on_pass":"allow","pass_data":false}]},"policy_id":"policy_id-0","policy_name":"pii","production_at":"NOW","updated_at":"NOW","updated_by":"admin","version_number":1,"version_status":"production"}"#;

const REGISTERED_FULL: &str = r#"{"inherit":"base","description":"masks pii","guardrails":{"add":["presidio"],"remove":["pii-legacy"]},"condition":{"model":"gpt-4"},"pipeline":{"mode":"post_call","steps":[{"guardrail":"presidio","on_fail":"block","on_pass":"allow","on_error":null,"pass_data":false,"modify_response_message":null}]}}"#;

const STORED_BARE: &str = r#"{"created_at":"NOW","guardrails_add":[],"guardrails_remove":[],"is_latest":true,"policy_id":"policy_id-0","policy_name":"pii","production_at":"NOW","updated_at":"NOW","version_number":1,"version_status":"production"}"#;

const REGISTERED_BARE: &str = r#"{"inherit":null,"description":null,"guardrails":{"add":[],"remove":[]},"condition":null,"pipeline":null}"#;

#[rstest]
#[case::full(FULL_REQUEST, Some("admin"), STORED_FULL, REGISTERED_FULL)]
#[case::bare(r#"{"policy_name": "pii"}"#, None, STORED_BARE, REGISTERED_BARE)]
#[tokio::test]
async fn writes_a_version_one_production_row_and_registers_it(
    #[case] body: &str,
    #[case] created_by: Option<&str>,
    #[case] stored: &str,
    #[case] registered: &str,
) {
    let fixture = Fixture::new(&[], &[]);
    let request: PolicyCreateRequest = serde_json::from_str(body).unwrap();
    let response = add_policy_to_db(&fixture.policy_store(), &request, created_by)
        .await
        .unwrap();
    let returned = (
        response.policy_id.as_str(),
        response.version_status.as_str(),
        response.created_by.as_deref(),
    );
    assert_eq!(returned, ("policy_id-0", "production", created_by));
    let rows = serde_json::to_string(&fixture.client.policies.rows()).unwrap();
    assert_eq!(rows, format!("[{}]", stored.replace("NOW", NOW)));
    let registry = fixture.policies.into_inner().unwrap();
    assert_eq!(
        serde_json::to_string(&registry.policies["pii"]).unwrap(),
        registered
    );
    assert_eq!(
        (registry.sources.get("pii"), registry.initialized),
        (Some(&PolicySource::Db), true)
    );
}

#[tokio::test]
async fn rejects_a_pipeline_without_steps_before_writing() {
    let fixture = Fixture::new(&[], &[]);
    let body = r#"{"policy_name": "p", "pipeline": {"mode": "pre_call", "steps": []}}"#;
    let request: PolicyCreateRequest = serde_json::from_str(body).unwrap();
    let error = add_policy_to_db(&fixture.policy_store(), &request, None)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Error adding policy to DB: List should have at least 1 item")
    );
    assert!(fixture.client.policies.calls().is_empty());
}

#[tokio::test]
async fn wraps_and_logs_a_failed_insert() {
    let (logs, _guard) = CapturedLogs::install();
    let fixture = Fixture::new(&[], &[]);
    fixture.client.policies.fail("create");
    let request: PolicyCreateRequest = serde_json::from_str(r#"{"policy_name": "p"}"#).unwrap();
    let error = add_policy_to_db(&fixture.policy_store(), &request, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error adding policy to DB: table refused create"
    );
    assert_eq!(
        logs.at(Level::ERROR),
        ["Error adding policy to DB: table refused create"]
    );
    assert!(!fixture.policies.into_inner().unwrap().initialized);
}
