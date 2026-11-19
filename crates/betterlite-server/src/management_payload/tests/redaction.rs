#![cfg(test)]

use betterlite_db::managed_resource_rows::mcp::{McpEnvVar, McpEnvVarScope, McpServerTable};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::json;

use crate::management_payload::redaction::{
    object_list, object_mapping, redact_env_var_values, redact_model_env_vars,
    redact_record_env_vars, redacted_env_var,
};
use crate::management_payload::tests::support::{json_object, json_value};

fn sample_response() -> JsonObject {
    json_object(json!({
        "env_vars": [
            {"name": "TOP", "value": "top-secret", "scope": "global"}
        ],
        "items": [
            {
                "server_id": "alpha",
                "env_vars": [
                    {"name": "ALPHA", "value": "alpha-secret", "scope": "user"}
                ],
                "nested": {
                    "env_vars": [
                        {"name": "DEEP", "value": "deep-secret", "scope": "global"}
                    ]
                },
                "items": [
                    {"env_vars": [{"name": "INNER", "value": "inner-secret"}]}
                ]
            },
            {
                "server_id": "beta",
                "env_vars": [
                    {"name": "BETA", "value": "beta-secret", "scope": "global"}
                ]
            }
        ]
    }))
}

#[test]
fn dict_and_list_only() {
    assert!(object_mapping(&json_value(json!({"a": 1}))).is_some());
    assert!(object_mapping(&json_value(json!("text"))).is_none());
    assert!(object_mapping(&json_value(json!([1, 2]))).is_none());
    assert!(object_mapping(&json_value(json!(null))).is_none());
    assert!(object_list(&json_value(json!([1, 2]))).is_some());
    assert!(object_list(&json_value(json!({"a": 1}))).is_none());
    assert!(object_list(&json_value(json!(7))).is_none());

    // a record whose env_vars is not a list keeps the value it carried
    let record = json_value(json!({
        "server_id": "s1",
        "env_vars": {"name": "A", "value": "secret"}
    }));
    assert_eq!(redact_record_env_vars(&record), record);

    // a record that is not an object is not a dict record
    let items = json_value(json!(["plain", 3]));
    assert_eq!(redact_record_env_vars(&items), items);
}

#[test]
fn env_entry_projection_and_nulls() {
    let entry = json_value(json!({
        "name": "API_KEY",
        "value": "sk-live",
        "scope": "global",
        "description": "the upstream key",
        "extra": "dropped"
    }));
    assert_eq!(
        redacted_env_var(&entry),
        json_value(json!({
            "name": "API_KEY",
            "scope": "global",
            "description": "the upstream key",
            "value": ""
        }))
    );
    // absent attributes project nulls, and nothing is invented for them
    assert_eq!(
        redacted_env_var(&json_value(json!({"name": "ONLY_NAME"}))),
        json_value(json!({
            "name": "ONLY_NAME",
            "scope": null,
            "description": null,
            "value": ""
        }))
    );
}

#[test]
fn env_entry_keeps_given_values() {
    // an empty name and an empty description are kept as given
    assert_eq!(
        redacted_env_var(&json_value(
            json!({"name": "", "scope": "user", "description": ""})
        )),
        json_value(json!({"name": "", "scope": "user", "description": "", "value": ""}))
    );
    // a value of the wrong type is kept as the entry carried it
    assert_eq!(
        redacted_env_var(&json_value(json!({"name": 7, "value": "secret"}))),
        json_value(json!({"name": 7, "scope": null, "description": null, "value": ""}))
    );
    // an entry that is not an object has none of the attributes
    assert_eq!(
        redacted_env_var(&json_value(json!("not-an-entry"))),
        json_value(json!({"name": null, "scope": null, "description": null, "value": ""}))
    );
}

#[test]
fn dict_and_model_copy() {
    let record = json_value(json!({"env_vars": [{"name": "A", "value": "secret"}]}));
    let before = record.clone();
    assert_eq!(
        redact_record_env_vars(&record),
        json_value(json!({
            "env_vars": [{"name": "A", "scope": null, "description": null, "value": ""}]
        }))
    );
    // the caller's record is a shallow copy source, never a target
    assert_eq!(record, before);

    let model: McpServerTable = serde_json::from_value(json!({
        "server_id": "s1",
        "transport": "sse",
        "env_vars": [{"name": "A", "value": "secret"}]
    }))
    .expect("the MCP server record decodes");
    let redacted = redact_model_env_vars(&model);
    assert_eq!(
        redacted.env_vars,
        Some(vec![McpEnvVar {
            name: "A".to_owned(),
            value: String::new(),
            scope: McpEnvVarScope::Global,
            description: None,
        }])
    );
    // the model the caller passed still carries the secret
    assert_eq!(
        model.env_vars.expect("the fixture has env vars")[0].value,
        "secret"
    );
}

#[test]
fn model_copy_without_env_vars() {
    let bare: McpServerTable = serde_json::from_value(json!({
        "server_id": "s2",
        "transport": "http"
    }))
    .expect("the MCP server record decodes");
    assert!(redact_model_env_vars(&bare).env_vars.is_none());
}

#[rstest]
#[case::null(json_value(json!(null)))]
#[case::boolean(json_value(json!(true)))]
#[case::number(json_value(json!(12)))]
#[case::text(json_value(json!("text")))]
#[case::list(json_value(json!(["env_vars", {"name": "A", "value": "secret"}])))]
fn arbitrary_object_unchanged(#[case] value: Json) {
    assert_eq!(redact_record_env_vars(&value), value);
}

#[test]
fn top_level_and_items_one_level() {
    let mut response = sample_response();
    redact_env_var_values(&mut response);

    assert_eq!(
        response["env_vars"],
        json_value(json!([
            {"name": "TOP", "scope": "global", "description": null, "value": ""}
        ]))
    );
    let items = object_list(&response["items"]).expect("items is a list");
    let first = object_mapping(&items[0]).expect("the first item is an object");
    assert_eq!(
        first["env_vars"],
        json_value(json!([
            {"name": "ALPHA", "scope": "user", "description": null, "value": ""}
        ]))
    );
    let second = object_mapping(&items[1]).expect("the second item is an object");
    assert_eq!(
        second["env_vars"],
        json_value(json!([
            {"name": "BETA", "scope": "global", "description": null, "value": ""}
        ]))
    );
    // only one level: the nested record and the items inside the item keep their values
    assert_eq!(
        first["nested"],
        json_value(json!({
            "env_vars": [{"name": "DEEP", "value": "deep-secret", "scope": "global"}]
        }))
    );
    assert_eq!(
        first["items"],
        json_value(json!([{"env_vars": [{"name": "INNER", "value": "inner-secret"}]}]))
    );
}

#[test]
fn empty_and_malformed_slots() {
    // empty lists stay empty
    let mut empty = json_object(json!({"env_vars": [], "items": []}));
    redact_env_var_values(&mut empty);
    assert_eq!(empty, json_object(json!({"env_vars": [], "items": []})));
    // a slot that is not a list is left as it was
    let mut malformed = json_object(json!({
        "env_vars": "not-a-list",
        "items": {"env_vars": [{"name": "A", "value": "secret"}]}
    }));
    let original = malformed.clone();
    redact_env_var_values(&mut malformed);
    assert_eq!(malformed, original);
    // an absent slot is not added
    let mut absent = json_object(json!({"server_id": "s1"}));
    redact_env_var_values(&mut absent);
    assert_eq!(absent, json_object(json!({"server_id": "s1"})));
}
