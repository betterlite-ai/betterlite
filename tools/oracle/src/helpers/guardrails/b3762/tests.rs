#![cfg(test)]
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_verdict_enforcement::api_error::{
    ApiErrorDetail, ApiErrorOutcome,
};
use betterlite_guardrails::cisco_verdict_enforcement::finalize::FinalizeOutcome;
use serde_json::json;

use super::support::{json_to_value, outcome_tag};

#[test]
fn json_to_value_round_trips_every_variant() {
    assert_eq!(json_to_value(&Json::Null), json!(null));
    assert_eq!(json_to_value(&Json::Bool(true)), json!(true));
    assert_eq!(
        json_to_value(&Json::Number("3.5".to_owned())),
        json!(3.5_f64)
    );
    assert_eq!(json_to_value(&Json::Text("hi".to_owned())), json!("hi"));
    assert_eq!(
        json_to_value(&Json::parse(r#"{"a":1}"#)),
        json!({"a": 1_i64})
    );
    assert_eq!(json_to_value(&Json::parse("[1,2]")), json!([1_i64, 2_i64]));
}

#[test]
fn outcome_tag_classifies_every_finalize_branch() {
    assert_eq!(
        outcome_tag(&FinalizeOutcome::Allow),
        json!({"tag": "allow"})
    );
    assert_eq!(
        outcome_tag(&FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned)),
        json!({"tag": "unscanned"})
    );
    let blocked = FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(ApiErrorDetail {
        error: "Cisco AI Defense guardrail unavailable",
        message: "down".to_owned(),
        error_type: "CiscoAIDefenseGuardrailAPIError".to_owned(),
    }));
    assert_eq!(
        outcome_tag(&blocked),
        json!({
            "tag": "api_error_blocked",
            "detail": {
                "error": "Cisco AI Defense guardrail unavailable",
                "message": "down",
                "error_type": "CiscoAIDefenseGuardrailAPIError",
            },
        })
    );
}
