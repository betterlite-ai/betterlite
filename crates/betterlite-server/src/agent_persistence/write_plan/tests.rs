#![cfg(test)]

use betterlite_core::agents::AgentResponse;
use betterlite_redact::structured::SensitiveDataMasker;
use chrono::DateTime;
use serde_json::json;

use super::PlanContext;
use super::plan_create;
use super::plan_patch;
use super::plan_replace;
use crate::agent_credentials::REDACTED_BY_LITELM;
use crate::agent_persistence::support::agent;
use crate::agent_persistence::support::patch;
use crate::agent_rows::codec::StoredAgent;

const STORED_AGENT: &str =
    r#"{"agent_id": "agent-stored", "agent_name": "stored-agent", "agent_card_params": {}}"#;

fn stored() -> StoredAgent {
    let decoded: AgentResponse =
        serde_json::from_str(STORED_AGENT).expect("the stored agent JSON parses");
    StoredAgent {
        agent: decoded,
        object_permission_id: None,
        litellm_params_column: Some(json!({"api_key": "sk-stored", "model": "m"})),
        kill_switch_column: Some(json!({
            "url": "https://example.test/hook",
            "method": "POST",
            "headers": {},
            "query_params": {},
            "body": null,
            "auth": {"type": "bearer", "token": "stored-token"}
        })),
    }
}

fn context(masker: &SensitiveDataMasker) -> PlanContext<'_> {
    PlanContext {
        masker,
        max_json_depth: 100,
        now: DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant"),
    }
}

fn marker_params() -> String {
    format!(r#"{{"api_key": "{REDACTED_BY_LITELM}", "model": "m"}}"#)
}

#[test]
fn create_drops_a_marker_secret_and_writes_an_absent_switch_as_null() {
    let masker = SensitiveDataMasker::default();
    let config = agent(&format!(
        r#"{{"agent_name": "a", "litellm_params": {}}}"#,
        marker_params()
    ));
    let plan = plan_create(&config, "author", &context(&masker)).expect("the create plan");
    assert_eq!(
        plan.columns.litellm_params.as_deref(),
        Some(r#"{"model":"m"}"#)
    );
    assert_eq!(plan.columns.kill_switch.as_deref(), Some("null"));
    assert_eq!(plan.columns.agent_card_params.as_deref(), Some("{}"));
    assert_eq!(plan.columns.static_headers, None);
    assert_eq!(plan.columns.extra_headers, None);
    assert_eq!(plan.columns.created_by.as_deref(), Some("author"));
    assert_eq!(plan.columns.updated_by.as_deref(), Some("author"));
}

#[test]
fn create_writes_static_headers_only_when_non_empty() {
    let masker = SensitiveDataMasker::default();
    let empty = agent(r#"{"agent_name": "a", "static_headers": {}}"#);
    let filled = agent(r#"{"agent_name": "a", "static_headers": {"x": "1"}}"#);
    let empty_plan = plan_create(&empty, "author", &context(&masker)).expect("the empty plan");
    let filled_plan = plan_create(&filled, "author", &context(&masker)).expect("the filled plan");
    assert_eq!(empty_plan.columns.static_headers, None);
    assert_eq!(
        filled_plan.columns.static_headers.as_deref(),
        Some(r#"{"x":"1"}"#)
    );
}

#[test]
fn create_deduplicates_access_group_ids_and_skips_null_limits() {
    let masker = SensitiveDataMasker::default();
    let config = agent(
        r#"{"agent_name": "a", "access_group_ids": ["b", "a", "b"], "tpm_limit": null, "rpm_limit": 7}"#,
    );
    let plan = plan_create(&config, "author", &context(&masker)).expect("the create plan");
    assert_eq!(
        plan.columns.access_group_ids,
        Some(vec![String::from("b"), String::from("a")])
    );
    assert_eq!(plan.columns.tpm_limit, None);
    assert_eq!(plan.columns.rpm_limit, Some(Some(7)));
}

#[test]
fn replace_restores_stored_secrets_and_clears_omitted_columns() {
    let masker = SensitiveDataMasker::default();
    let incoming = agent(&format!(
        r#"{{"agent_name": "a", "agent_card_params": {{}}, "litellm_params": {{"api_key": "{REDACTED_BY_LITELM}", "model": "m2"}}}}"#
    ));
    let existing = stored();
    let plan =
        plan_replace(&incoming, Some(&existing), "author", &context(&masker)).expect("the plan");
    assert_eq!(
        plan.columns.litellm_params.as_deref(),
        Some(r#"{"api_key":"sk-stored","model":"m2"}"#)
    );
    assert_eq!(plan.columns.kill_switch.as_deref(), Some("null"));
    assert_eq!(plan.columns.static_headers.as_deref(), Some("{}"));
    assert_eq!(plan.columns.extra_headers, Some(Vec::new()));
    assert_eq!(plan.columns.access_group_ids, Some(Vec::new()));
    assert_eq!(plan.columns.tpm_limit, None);
    assert_eq!(plan.columns.agent_name.as_deref(), Some("a"));
    assert_eq!(plan.columns.created_by, None);
    assert_eq!(plan.columns.updated_by.as_deref(), Some("author"));
}

#[test]
fn replace_keeps_a_stored_secret_the_body_leaves_out() {
    let masker = SensitiveDataMasker::default();
    let incoming =
        agent(r#"{"agent_name": "a", "agent_card_params": {}, "litellm_params": {"model": "m"}}"#);
    let existing = stored();
    let plan =
        plan_replace(&incoming, Some(&existing), "author", &context(&masker)).expect("the plan");
    assert_eq!(
        plan.columns.litellm_params.as_deref(),
        Some(r#"{"api_key":"sk-stored","model":"m"}"#)
    );
}

#[test]
fn replace_restores_a_stored_kill_switch_secret() {
    let masker = SensitiveDataMasker::default();
    let incoming = agent(&format!(
        r#"{{"agent_name": "a", "agent_card_params": {{}}, "kill_switch": {{"url": "https://example.test/hook", "auth": {{"type": "bearer", "token": "{REDACTED_BY_LITELM}"}}}}}}"#
    ));
    let existing = stored();
    let plan =
        plan_replace(&incoming, Some(&existing), "author", &context(&masker)).expect("the plan");
    assert_eq!(
        plan.columns.kill_switch.as_deref(),
        Some(
            r#"{"auth":{"token":"stored-token","type":"bearer"},"body":null,"headers":{},"method":"POST","query_params":{},"url":"https://example.test/hook"}"#
        )
    );
}

#[test]
fn patch_writes_only_the_carried_columns() {
    let masker = SensitiveDataMasker::default();
    let existing = stored();
    let request = patch(r#"{"agent_name": "b"}"#);
    let plan = plan_patch(&request, &existing, "author", &context(&masker)).expect("the plan");
    assert_eq!(plan.columns.agent_name.as_deref(), Some("b"));
    assert_eq!(plan.columns.litellm_params, None);
    assert_eq!(plan.columns.agent_card_params, None);
    assert_eq!(plan.columns.static_headers, None);
    assert_eq!(plan.columns.extra_headers, None);
    assert_eq!(plan.columns.access_group_ids, None);
    assert_eq!(plan.columns.kill_switch, None);
    assert_eq!(plan.columns.tpm_limit, None);
    assert_eq!(plan.columns.updated_by.as_deref(), Some("author"));
}

#[test]
fn patch_turns_null_headers_and_lists_into_empty_and_clears_a_null_limit() {
    let masker = SensitiveDataMasker::default();
    let existing = stored();
    let request = patch(
        r#"{"static_headers": null, "extra_headers": null, "access_group_ids": null, "tpm_limit": null}"#,
    );
    let plan = plan_patch(&request, &existing, "author", &context(&masker)).expect("the plan");
    assert_eq!(plan.columns.static_headers.as_deref(), Some("{}"));
    assert_eq!(plan.columns.extra_headers, Some(Vec::new()));
    assert_eq!(plan.columns.access_group_ids, Some(Vec::new()));
    assert_eq!(plan.columns.tpm_limit, Some(None));
}

#[test]
fn patch_restores_a_marker_secret_and_the_stored_switch_secret() {
    let masker = SensitiveDataMasker::default();
    let existing = stored();
    let request = patch(&format!(r#"{{"litellm_params": {}}}"#, marker_params()));
    let plan = plan_patch(&request, &existing, "author", &context(&masker)).expect("the plan");
    assert_eq!(
        plan.columns.litellm_params.as_deref(),
        Some(r#"{"api_key":"sk-stored","model":"m"}"#)
    );
    let switch = patch(&format!(
        r#"{{"kill_switch": {{"url": "https://example.test/hook", "auth": {{"type": "bearer", "token": "{REDACTED_BY_LITELM}"}}}}}}"#
    ));
    let switch_plan =
        plan_patch(&switch, &existing, "author", &context(&masker)).expect("the switch plan");
    let stored_switch = switch_plan.columns.kill_switch.expect("the switch column");
    assert!(stored_switch.contains(r#""token":"stored-token""#));
}

#[test]
fn patch_writes_a_present_card_and_de_duplicates_groups() {
    let masker = SensitiveDataMasker::default();
    let existing = stored();
    let request =
        patch(r#"{"agent_card_params": {"name": "c"}, "access_group_ids": ["x", "x", "y"]}"#);
    let plan = plan_patch(&request, &existing, "author", &context(&masker)).expect("the plan");
    let card = plan.columns.agent_card_params.expect("the card column");
    assert!(card.contains(r#""name":"c""#));
    assert_eq!(
        plan.columns.access_group_ids,
        Some(vec![String::from("x"), String::from("y")])
    );
}
