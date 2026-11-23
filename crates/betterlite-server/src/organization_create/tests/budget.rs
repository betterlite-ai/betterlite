use std::collections::BTreeMap;

use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, PortError};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};

use super::support::parsed;
use crate::organization_create::budget::{
    BUDGET_RESET_AT_FIELD, BudgetTransformError, NESTED_BUDGET_FIELD, UPDATED_BY_FIELD,
    build_budget_write_data, handle_nested_budget_structure_in_organization_update_request,
};
use crate::organization_create::records::to_json_data;

fn data(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = parsed(text) else {
        panic!("the test body is a JSON object");
    };
    fields
}

struct FixedReset {
    at: Timestamp,
}

impl BudgetResetCalculator for FixedReset {
    fn reset_time(&self, budget_duration: &str) -> Result<Timestamp, PortError> {
        assert_eq!(budget_duration, "1d");
        Ok(self.at)
    }
}

struct PanicReset;

impl BudgetResetCalculator for PanicReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        panic!("a request without a duration must not compute a reset time");
    }
}

struct FailingReset;

impl BudgetResetCalculator for FailingReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("unknown budget duration"))
    }
}

#[test]
fn budget_duration_omitted() {
    let input = data(r#"{"max_budget":10,"created_by":"writer"}"#);
    let written = build_budget_write_data(&input, "actor", &PanicReset).unwrap();
    assert!(!written.contains_key(BUDGET_RESET_AT_FIELD));
    assert_eq!(written.get("max_budget"), Some(&parsed("10")));
    assert_eq!(
        written.get("created_by"),
        Some(&JsonData::String(String::from("writer")))
    );
    assert_eq!(
        written.get(UPDATED_BY_FIELD),
        Some(&JsonData::String(String::from("actor")))
    );
    // the input mapping is left as it came
    assert!(!input.contains_key(UPDATED_BY_FIELD));
}

#[test]
fn budget_duration_reset_and_clear() {
    let at: Timestamp = serde_json::from_str(r#""2026-02-03T04:05:06+00:00""#).unwrap();
    let expected = to_json_data(&at).unwrap();
    let written = build_budget_write_data(
        &data(r#"{"budget_duration":"1d"}"#),
        "actor",
        &FixedReset { at },
    )
    .unwrap();
    assert_eq!(written.get(BUDGET_RESET_AT_FIELD), Some(&expected));
    assert!(matches!(expected, JsonData::String(_)));
    assert_eq!(
        written.get("budget_duration"),
        Some(&JsonData::String(String::from("1d")))
    );

    // a sent nonstring or null duration clears the reset time without asking the calculator
    for body in [
        r#"{"budget_duration":null}"#,
        r#"{"budget_duration":3600}"#,
        r#"{"budget_duration":false}"#,
    ] {
        let cleared = build_budget_write_data(&data(body), "actor", &FailingReset).unwrap();
        assert_eq!(
            cleared.get(BUDGET_RESET_AT_FIELD),
            Some(&JsonData::Null),
            "{body}"
        );
    }

    let failed =
        build_budget_write_data(&data(r#"{"budget_duration":"1y"}"#), "actor", &FailingReset);
    assert!(matches!(failed, Err(BudgetTransformError::Reset(_))));
}

#[test]
fn budget_actor_override() {
    let written = build_budget_write_data(
        &data(r#"{"updated_by":"spoofed","created_by":"original","max_budget":1}"#),
        "actor",
        &PanicReset,
    )
    .unwrap();
    assert_eq!(
        written.get(UPDATED_BY_FIELD),
        Some(&JsonData::String(String::from("actor")))
    );
    assert_eq!(
        written.get("created_by"),
        Some(&JsonData::String(String::from("original")))
    );
}

#[test]
fn nested_budget_precedence() {
    let transformed = handle_nested_budget_structure_in_organization_update_request(data(
        r#"{
                "organization_alias": "acme",
                "max_budget": 10,
                "soft_budget": 7,
                "litellm_budget_table": {
                    "max_budget": 5,
                    "soft_budget": null,
                    "budget_id": "b1",
                    "model_max_budget": {"gpt-4": 9},
                    "unknown_field": 1
                }
            }"#,
    ))
    .unwrap();
    assert!(!transformed.contains_key(NESTED_BUDGET_FIELD));
    assert_eq!(transformed.get("max_budget"), Some(&parsed("5")));
    // a nested null wins over the top-level value, like the source dict update
    assert_eq!(transformed.get("soft_budget"), Some(&JsonData::Null));
    assert_eq!(
        transformed.get("budget_id"),
        Some(&JsonData::String(String::from("b1")))
    );
    assert_eq!(
        transformed.get("model_max_budget"),
        Some(&parsed(r#"{"gpt-4":9}"#))
    );
    // keys outside LiteLLM_BudgetTable never reach the top level
    assert!(!transformed.contains_key("unknown_field"));
    assert_eq!(
        transformed.get("organization_alias"),
        Some(&JsonData::String(String::from("acme")))
    );
}

#[test]
fn nested_budget_falsey_removed() {
    let untouched =
        handle_nested_budget_structure_in_organization_update_request(data(r#"{"max_budget":1}"#))
            .unwrap();
    assert_eq!(untouched, data(r#"{"max_budget":1}"#));

    for nested in ["{}", "[]", r#""""#, "0", "false", "null"] {
        let body = format!(r#"{{"max_budget":1,"{NESTED_BUDGET_FIELD}":{nested}}}"#);
        let transformed =
            handle_nested_budget_structure_in_organization_update_request(data(&body)).unwrap();
        assert_eq!(transformed, data(r#"{"max_budget":1}"#), "{nested}");
    }
}

#[test]
fn nested_budget_nonmapping_fail() {
    for nested in [r#""x""#, "[1]", "5", "true"] {
        let body = format!(r#"{{"max_budget":1,"{NESTED_BUDGET_FIELD}":{nested}}}"#);
        let failed = handle_nested_budget_structure_in_organization_update_request(data(&body));
        assert!(
            matches!(failed, Err(BudgetTransformError::NestedNotAMapping)),
            "{nested}"
        );
    }
}
