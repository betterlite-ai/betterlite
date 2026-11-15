#![cfg(test)]

use chrono::{DateTime, Utc};

use crate::pipeline::json_value::JsonValue;
use crate::records::JsonObject;
use crate::registry::PolicySource;

use super::{
    PolicyOrder, PolicyRow, PolicySort, PolicyVersionJson, PolicyVersionSourceRow,
    policy_db_response,
};

fn timestamp(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn row() -> PolicyRow {
    PolicyRow {
        policy_id: "policy_1".to_owned(),
        policy_name: "healthcare".to_owned(),
        version_number: Some(3),
        version_status: Some("draft".to_owned()),
        parent_version_id: Some("policy_0".to_owned()),
        is_latest: Some(false),
        published_at: Some(timestamp("2024-02-03T04:05:06Z")),
        production_at: Some(timestamp("2024-03-04T05:06:07Z")),
        inherit: Some("baseline".to_owned()),
        description: Some("HIPAA".to_owned()),
        guardrails_add: Some(names(&["hipaa", "audit"])),
        guardrails_remove: Some(names(&["pii"])),
        condition: Some(JsonObject::from([(
            "model".to_owned(),
            JsonValue::String("gpt-4".to_owned()),
        )])),
        pipeline: Some(JsonObject::from([(
            "mode".to_owned(),
            JsonValue::String("pre_call".to_owned()),
        )])),
        created_at: timestamp("2024-01-01T00:00:00Z"),
        updated_at: timestamp("2024-01-02T00:00:00Z"),
        created_by: Some("alice".to_owned()),
        updated_by: Some("bob".to_owned()),
    }
}

fn bare_row() -> PolicyRow {
    PolicyRow {
        policy_id: "policy_1".to_owned(),
        policy_name: "healthcare".to_owned(),
        version_number: None,
        version_status: None,
        parent_version_id: None,
        is_latest: None,
        published_at: None,
        production_at: None,
        inherit: None,
        description: None,
        guardrails_add: None,
        guardrails_remove: None,
        condition: None,
        pipeline: None,
        created_at: timestamp("2024-01-01T00:00:00Z"),
        updated_at: timestamp("2024-01-02T00:00:00Z"),
        created_by: None,
        updated_by: None,
    }
}

#[test]
fn adapter_applies_the_defaults_for_missing_version_columns() {
    let response = policy_db_response(&bare_row());
    assert_eq!(response.version_number, 1_i32);
    assert_eq!(response.version_status, "production");
    assert!(response.is_latest);
    assert_eq!(response.parent_version_id, None);
    assert_eq!(response.published_at, None);
    assert_eq!(response.production_at, None);
    assert_eq!(response.guardrails_add, Vec::<String>::new());
    assert_eq!(response.guardrails_remove, Vec::<String>::new());
    assert_eq!(response.definition_location, PolicySource::Db);
}

#[test]
fn adapter_copies_the_identity_and_version_columns() {
    let response = policy_db_response(&row());
    assert_eq!(response.policy_id, "policy_1");
    assert_eq!(response.policy_name, "healthcare");
    assert_eq!(response.version_number, 3_i32);
    assert_eq!(response.version_status, "draft");
    assert_eq!(response.parent_version_id.as_deref(), Some("policy_0"));
    assert!(!response.is_latest);
    assert_eq!(
        response.published_at,
        Some(timestamp("2024-02-03T04:05:06Z"))
    );
    assert_eq!(
        response.production_at,
        Some(timestamp("2024-03-04T05:06:07Z"))
    );
}

#[test]
fn adapter_copies_the_content_and_metadata_columns() {
    let response = policy_db_response(&row());
    assert_eq!(response.inherit.as_deref(), Some("baseline"));
    assert_eq!(response.description.as_deref(), Some("HIPAA"));
    assert_eq!(response.guardrails_add, ["hipaa", "audit"]);
    assert_eq!(response.guardrails_remove, ["pii"]);
    assert_eq!(response.condition, row().condition);
    assert_eq!(response.pipeline, row().pipeline);
    assert_eq!(response.created_at, Some(timestamp("2024-01-01T00:00:00Z")));
    assert_eq!(response.updated_at, Some(timestamp("2024-01-02T00:00:00Z")));
    assert_eq!(response.created_by.as_deref(), Some("alice"));
    assert_eq!(response.updated_by.as_deref(), Some("bob"));
    assert_eq!(response.definition_location, PolicySource::Db);
}

#[test]
fn row_reads_every_stored_column() {
    let parsed: PolicyRow = serde_json::from_str(
        r#"{
            "policy_id": "policy_1",
            "policy_name": "healthcare",
            "version_number": 3,
            "version_status": "draft",
            "parent_version_id": "policy_0",
            "is_latest": false,
            "published_at": "2024-02-03T04:05:06Z",
            "production_at": "2024-03-04T05:06:07Z",
            "inherit": "baseline",
            "description": "HIPAA",
            "guardrails_add": ["hipaa", "audit"],
            "guardrails_remove": ["pii"],
            "condition": {"model": "gpt-4"},
            "pipeline": {"mode": "pre_call"},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z",
            "created_by": "alice",
            "updated_by": "bob"
        }"#,
    )
    .unwrap();
    assert_eq!(parsed, row());
}

#[test]
fn row_defaults_the_columns_a_partial_row_omits() {
    let parsed: PolicyRow = serde_json::from_str(
        r#"{
            "policy_id": "policy_1",
            "policy_name": "healthcare",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-02T00:00:00Z"
        }"#,
    )
    .unwrap();
    assert_eq!(parsed.version_number, None);
    assert_eq!(parsed.version_status, None);
    assert_eq!(parsed.is_latest, None);
    assert_eq!(parsed.guardrails_add, None);
}

#[test]
fn row_requires_the_ids_and_the_timestamps() {
    let missing_id = r#"{"policy_name":"n","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}"#;
    serde_json::from_str::<PolicyRow>(missing_id).unwrap_err();
    let missing_time = r#"{"policy_id":"p","policy_name":"n","created_at":"2024-01-01T00:00:00Z"}"#;
    serde_json::from_str::<PolicyRow>(missing_time).unwrap_err();
}

fn version_row(condition: &str) -> PolicyVersionSourceRow {
    serde_json::from_str(&format!(
        r#"{{
            "policy_id": "policy_1",
            "policy_name": "healthcare",
            "version_number": 2,
            "inherit": "baseline",
            "guardrails_add": ["hipaa"],
            "condition": {condition}
        }}"#
    ))
    .unwrap()
}

#[test]
fn version_source_row_reads_an_object_condition() {
    let parsed = version_row(r#"{"model": "gpt-4"}"#);
    assert_eq!(parsed.version_number, 2_i32);
    assert_eq!(parsed.inherit.as_deref(), Some("baseline"));
    assert_eq!(parsed.guardrails_add, Some(names(&["hipaa"])));
    assert_eq!(parsed.guardrails_remove, None);
    assert_eq!(
        parsed.condition,
        Some(PolicyVersionJson::Object(JsonObject::from([(
            "model".to_owned(),
            JsonValue::String("gpt-4".to_owned()),
        )])))
    );
    assert_eq!(parsed.pipeline, None);
}

#[test]
fn version_source_row_reads_a_text_condition() {
    let parsed = version_row(r#""{\"model\": \"gpt-4\"}""#);
    assert_eq!(
        parsed.condition,
        Some(PolicyVersionJson::Text(r#"{"model": "gpt-4"}"#.to_owned()))
    );
}

#[test]
fn sort_serializes_as_the_prisma_order_words() {
    let order = PolicyOrder::from([("version_number".to_owned(), PolicySort::Desc)]);
    assert_eq!(
        serde_json::to_string(&order).unwrap(),
        r#"{"version_number":"desc"}"#
    );
    let ascending = PolicyOrder::from([("created_at".to_owned(), PolicySort::Asc)]);
    assert_eq!(
        serde_json::to_string(&ascending).unwrap(),
        r#"{"created_at":"asc"}"#
    );
}
