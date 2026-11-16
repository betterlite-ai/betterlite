#![cfg(test)]

use rust_decimal::Decimal;

use super::{GuardrailRecord, PolicyRecord, UsageCounterRow, UsageMetricRow};

#[test]
fn usage_counter_row_decodes_a_priced_row() {
    let source = r#"{
        "usage_unit": "requests",
        "guardrail_id": "uuid-1",
        "team_id": "team-a",
        "api_key": "key-1",
        "units": 12,
        "untracked_units": 3,
        "cost": 1.75
    }"#;
    let row: UsageCounterRow = serde_json::from_str(source).unwrap();
    assert_eq!(row.usage_unit, "requests");
    assert_eq!(row.guardrail_id, "uuid-1");
    assert_eq!(row.team_id, "team-a");
    assert_eq!(row.api_key, "key-1");
    assert_eq!(row.units, 12);
    assert_eq!(row.untracked_units, 3);
    assert_eq!(row.cost, Some(Decimal::new(175, 2)));
}

#[test]
fn usage_counter_row_reads_a_null_cost_as_unpriced() {
    let source = r#"{
        "usage_unit": "requests",
        "guardrail_id": "uuid-1",
        "team_id": "team-a",
        "api_key": "key-1",
        "units": 12,
        "untracked_units": 0,
        "cost": null
    }"#;
    let row: UsageCounterRow = serde_json::from_str(source).unwrap();
    assert_eq!(row.cost, None);
}

#[test]
fn usage_counter_row_defaults_an_absent_cost() {
    let source = r#"{
        "usage_unit": "tokens",
        "guardrail_id": "uuid-1",
        "team_id": "",
        "api_key": "",
        "units": 0,
        "untracked_units": 0
    }"#;
    let row: UsageCounterRow = serde_json::from_str(source).unwrap();
    assert_eq!(row.cost, None);
}

#[test]
fn usage_metric_row_defaults_absent_and_null_counters() {
    let present = r#"{
        "date": "2026-11-01",
        "id": "lakera",
        "requests_evaluated": 10,
        "passed_count": 8,
        "blocked_count": 2,
        "flagged_count": 1
    }"#;
    let row: UsageMetricRow = serde_json::from_str(present).unwrap();
    assert_eq!(row.date, "2026-11-01");
    assert_eq!(row.id, "lakera");
    assert_eq!(row.requests_evaluated, Some(10));
    assert_eq!(row.passed_count, Some(8));
    assert_eq!(row.blocked_count, Some(2));
    assert_eq!(row.flagged_count, Some(1));
    let absent: UsageMetricRow =
        serde_json::from_str(r#"{"date": "2026-11-01", "id": "lakera"}"#).unwrap();
    assert_eq!(absent.requests_evaluated, None);
    assert_eq!(absent.passed_count, None);
    assert_eq!(absent.blocked_count, None);
    assert_eq!(absent.flagged_count, None);
    let null: UsageMetricRow = serde_json::from_str(
        r#"{"date": "2026-11-01", "id": "lakera", "requests_evaluated": null, "blocked_count": null}"#,
    )
    .unwrap();
    assert_eq!(null.requests_evaluated, None);
    assert_eq!(null.blocked_count, None);
}

#[test]
fn guardrail_record_decodes_a_config_block() {
    let source = r#"{
        "guardrail_id": "uuid-1",
        "guardrail_name": "lakera",
        "litellm_params": {"guardrail": "lakera", "mode": "pre_call"},
        "guardrail_info": {"type": "pii", "id": "pii-1"}
    }"#;
    let record: GuardrailRecord = serde_json::from_str(source).unwrap();
    assert_eq!(record.guardrail_id.as_deref(), Some("uuid-1"));
    assert_eq!(record.guardrail_name.as_deref(), Some("lakera"));
    assert_eq!(record.litellm_params.guardrail.as_deref(), Some("lakera"));
    assert_eq!(
        record.guardrail_info.as_ref().map(|info| info.0.len()),
        Some(2)
    );
}

#[test]
fn guardrail_record_defaults_absent_fields() {
    let record: GuardrailRecord = serde_json::from_str(r#"{"guardrail_id": "uuid-1"}"#).unwrap();
    assert_eq!(record.guardrail_id.as_deref(), Some("uuid-1"));
    assert_eq!(record.guardrail_name, None);
    assert_eq!(record.litellm_params.guardrail, None);
    assert_eq!(record.guardrail_info, None);
    let bare: GuardrailRecord = serde_json::from_str("{}").unwrap();
    assert_eq!(bare.guardrail_id, None);
}

#[test]
fn guardrail_record_reads_null_fields_as_absent() {
    let source = r#"{
        "guardrail_id": "uuid-1",
        "guardrail_name": null,
        "litellm_params": {"guardrail": null},
        "guardrail_info": null
    }"#;
    let record: GuardrailRecord = serde_json::from_str(source).unwrap();
    assert_eq!(record.guardrail_name, None);
    assert_eq!(record.litellm_params.guardrail, None);
    assert_eq!(record.guardrail_info, None);
}

#[test]
fn policy_record_decodes_a_named_policy() {
    let record: PolicyRecord =
        serde_json::from_str(r#"{"policy_id": "pol-1", "policy_name": "PII policy"}"#).unwrap();
    assert_eq!(record.policy_id, "pol-1");
    assert_eq!(record.policy_name.as_deref(), Some("PII policy"));
}

#[test]
fn policy_record_defaults_an_absent_name() {
    let record: PolicyRecord = serde_json::from_str(r#"{"policy_id": "pol-1"}"#).unwrap();
    assert_eq!(record.policy_name, None);
    let unnamed: PolicyRecord =
        serde_json::from_str(r#"{"policy_id": "pol-1", "policy_name": null}"#).unwrap();
    assert_eq!(unnamed.policy_name, None);
}
