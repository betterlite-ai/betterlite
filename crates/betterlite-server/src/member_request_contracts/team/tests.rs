#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde::Serialize;
use serde_json::json;

use super::DeleteTeamRequest;
use crate::member_request_contracts::MemberContractError;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn raw<T: Serialize>(body: T) -> Json {
    serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap()
}

#[rstest]
#[case("[\"a\", \"b\", \"a\", \"c\", \"b\"]", vec!["a", "b", "c"])]
#[case("[\"a\"]", vec!["a"])]
#[case("[]", vec![])]
#[case("[\"a\", \"a\", \"a\"]", vec!["a"])]
fn keeps_the_first_occurrence_of_each_team_id(#[case] team_ids: &str, #[case] expected: Vec<&str>) {
    let request =
        DeleteTeamRequest::from_value(&raw(json!({ "team_ids": parse(team_ids) }))).unwrap();
    assert_eq!(request.team_ids, expected);
}

#[test]
fn rejects_a_body_without_team_ids() {
    let error = DeleteTeamRequest::from_value(&raw(json!({}))).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn rejects_a_null_team_ids() {
    let error = DeleteTeamRequest::from_value(&raw(json!({ "team_ids": null }))).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[rstest]
#[case("\"team\"")]
#[case("1")]
#[case("{}")]
#[case("[\"a\", 1]")]
fn rejects_team_ids_that_is_not_a_list_of_strings(#[case] team_ids: &str) {
    let error =
        DeleteTeamRequest::from_value(&raw(json!({ "team_ids": parse(team_ids) }))).unwrap_err();
    assert!(matches!(error, MemberContractError::Decode { .. }));
}

#[test]
fn ignores_unknown_keys() {
    let request =
        DeleteTeamRequest::from_value(&raw(json!({ "team_ids": ["a"], "team_alias": "x" })))
            .unwrap();
    assert_eq!(request.team_ids, vec!["a"]);
}

#[rstest]
#[case("null")]
#[case("[]")]
#[case("\"team\"")]
#[case("1")]
fn rejects_a_body_that_is_not_an_object(#[case] body: &str) {
    let error = DeleteTeamRequest::from_value(&parse(body)).unwrap_err();
    assert!(matches!(
        error,
        MemberContractError::Validation { message } if message == "Input should be a valid dictionary or object to extract fields from"
    ));
}
