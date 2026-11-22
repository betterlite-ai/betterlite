#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;

use super::{TokenValidationFailure, validate_token_response};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn rules(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

fn expected_failure(field: &str, message: &str) -> TokenValidationFailure {
    TokenValidationFailure {
        server_id: "srv".to_owned(),
        field: field.to_owned(),
        message: message.to_owned(),
    }
}

#[test]
fn accepts_an_empty_rule_map() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"access_token":"token"}"#),
            &BTreeMap::new(),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn accepts_an_empty_rule_map_for_a_non_object_token() {
    assert_eq!(
        validate_token_response(&json("[]"), &BTreeMap::new(), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_string_field() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"scope":"read"}"#),
            &rules(r#"{"scope":"read"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn reports_a_string_mismatch() {
    let failure = validate_token_response(
        &json(r#"{"scope":"write"}"#),
        &rules(r#"{"scope":"read"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "scope",
            "OAuth token rejected: 'scope' = 'write', expected 'read'"
        )
    );
}

#[test]
fn compares_strings_case_sensitively() {
    let failure = validate_token_response(
        &json(r#"{"scope":"READ"}"#),
        &rules(r#"{"scope":"read"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "scope",
            "OAuth token rejected: 'scope' = 'READ', expected 'read'"
        )
    );
}

#[test]
fn matches_an_integer_with_a_string_rule() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"org_id":12345}"#),
            &rules(r#"{"org_id":"12345"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn reports_an_integer_mismatch() {
    let failure = validate_token_response(
        &json(r#"{"org_id":12345}"#),
        &rules(r#"{"org_id":"12346"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "org_id",
            "OAuth token rejected: 'org_id' = '12345', expected '12346'"
        )
    );
}

#[test]
fn matches_a_negative_zero_integer() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":-0}"#), &rules(r#"{"n":"0"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_big_integer_beyond_i64() {
    let digits = "123456789012345678901234567890";
    let token = json(r#"{"n":123456789012345678901234567890}"#);
    assert_eq!(
        validate_token_response(&token, &rules(&format!(r#"{{"n":"{digits}"}}"#)), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_whole_float_with_its_dotted_text() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"expires_in":300.0}"#),
            &rules(r#"{"expires_in":"300.0"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn reports_a_whole_float_mismatch() {
    let failure = validate_token_response(
        &json(r#"{"expires_in":300.0}"#),
        &rules(r#"{"expires_in":300}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "expires_in",
            "OAuth token rejected: 'expires_in' = '300.0', expected '300'"
        )
    );
}

#[test]
fn matches_a_negative_zero_float() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":-0.0}"#), &rules(r#"{"n":"-0.0"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_zero_float() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":0.0}"#), &rules(r#"{"n":"0.0"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_small_float_in_python_exponent_form() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":1e-7}"#), &rules(r#"{"n":"1e-07"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn reports_a_small_float_message() {
    let failure = validate_token_response(
        &json(r#"{"n":1e-7}"#),
        &rules(r#"{"n":"0.0000001"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "n",
            "OAuth token rejected: 'n' = '1e-07', expected '0.0000001'"
        )
    );
}

#[test]
fn matches_a_large_float_in_python_exponent_form() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":1e20}"#), &rules(r#"{"n":"1e+20"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_the_plain_float_notation_boundaries() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":0.0001}"#), &rules(r#"{"n":"0.0001"}"#), "srv"),
        Ok(())
    );
    assert_eq!(
        validate_token_response(
            &json(r#"{"n":9999999999999998.0}"#),
            &rules(r#"{"n":"9999999999999998.0"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn matches_the_scientific_notation_boundary() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":1e16}"#), &rules(r#"{"n":"1e+16"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_an_overflowing_float_as_infinity() {
    assert_eq!(
        validate_token_response(&json(r#"{"n":1e400}"#), &rules(r#"{"n":"inf"}"#), "srv"),
        Ok(())
    );
    assert_eq!(
        validate_token_response(&json(r#"{"n":-1e400}"#), &rules(r#"{"n":"-inf"}"#), "srv"),
        Ok(())
    );
}

#[test]
fn matches_a_bool_with_its_json_spelling() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"verified":true}"#),
            &rules(r#"{"verified":"true"}"#),
            "srv"
        ),
        Ok(())
    );
    assert_eq!(
        validate_token_response(
            &json(r#"{"verified":false}"#),
            &rules(r#"{"verified":"false"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn matches_a_bool_against_a_bool_rule() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"verified":true}"#),
            &rules(r#"{"verified":true}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn rejects_the_python_spelling_of_a_bool() {
    let failure = validate_token_response(
        &json(r#"{"verified":true}"#),
        &rules(r#"{"verified":"True"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "verified",
            "OAuth token rejected: 'verified' = 'True', expected 'True'"
        )
    );
}

#[test]
fn treats_a_null_token_value_as_absent() {
    let failure = validate_token_response(
        &json(r#"{"org_id":null}"#),
        &rules(r#"{"org_id":"1"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "org_id",
            "OAuth token rejected: required field 'org_id' is absent"
        )
    );
}

#[test]
fn reports_an_absent_field() {
    let failure = validate_token_response(
        &json(r#"{"access_token":"token"}"#),
        &rules(r#"{"org_id":"1"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "org_id",
            "OAuth token rejected: required field 'org_id' is absent"
        )
    );
}

#[test]
fn treats_a_non_object_token_as_all_absent() {
    let failure = validate_token_response(&json(r#""text""#), &rules(r#"{"org_id":"1"}"#), "srv")
        .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "org_id",
            "OAuth token rejected: required field 'org_id' is absent"
        )
    );
}

#[test]
fn walks_a_dot_path() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"team":{"enterprise_id":"e1"}}"#),
            &rules(r#"{"team.enterprise_id":"e1"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn prefers_a_whole_dotted_key() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"team":{"enterprise_id":"nested"},"team.enterprise_id":"flat"}"#),
            &rules(r#"{"team.enterprise_id":"flat"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn walks_when_the_whole_dotted_key_is_null() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"team":{"enterprise_id":"nested"},"team.enterprise_id":null}"#),
            &rules(r#"{"team.enterprise_id":"nested"}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn stops_the_dot_walk_at_a_non_object() {
    let failure = validate_token_response(
        &json(r#"{"team":"text"}"#),
        &rules(r#"{"team.enterprise_id":"e1"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "team.enterprise_id",
            "OAuth token rejected: required field 'team.enterprise_id' is absent"
        )
    );
}

#[test]
fn stops_the_dot_walk_at_a_missing_part() {
    let failure = validate_token_response(
        &json(r#"{"team":{}}"#),
        &rules(r#"{"team.enterprise_id":"e1"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "team.enterprise_id",
            "OAuth token rejected: required field 'team.enterprise_id' is absent"
        )
    );
}

#[test]
fn treats_a_null_dot_path_end_as_absent() {
    let failure = validate_token_response(
        &json(r#"{"team":{"enterprise_id":null}}"#),
        &rules(r#"{"team.enterprise_id":"e1"}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "team.enterprise_id",
            "OAuth token rejected: required field 'team.enterprise_id' is absent"
        )
    );
}

#[test]
fn reports_the_first_failing_rule_in_key_order() {
    let failure =
        validate_token_response(&json("{}"), &rules(r#"{"z":"1","a":"x"}"#), "srv").unwrap_err();
    assert_eq!(
        failure,
        expected_failure("a", "OAuth token rejected: required field 'a' is absent")
    );
}

#[test]
fn matches_a_container_by_its_json_text() {
    assert_eq!(
        validate_token_response(
            &json(r#"{"scopes":["a","b"]}"#),
            &rules(r#"{"scopes":["a","b"]}"#),
            "srv"
        ),
        Ok(())
    );
}

#[test]
fn reports_a_null_rule_as_none() {
    let failure =
        validate_token_response(&json(r#"{"x":"anything"}"#), &rules(r#"{"x":null}"#), "srv")
            .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "x",
            "OAuth token rejected: 'x' = 'anything', expected 'None'"
        )
    );
}

#[test]
fn reports_a_container_mismatch_in_json_text() {
    let failure = validate_token_response(
        &json(r#"{"scopes":["a"]}"#),
        &rules(r#"{"scopes":["b"]}"#),
        "srv",
    )
    .unwrap_err();
    assert_eq!(
        failure,
        expected_failure(
            "scopes",
            r#"OAuth token rejected: 'scopes' = '["a"]', expected '["b"]'"#
        )
    );
}
