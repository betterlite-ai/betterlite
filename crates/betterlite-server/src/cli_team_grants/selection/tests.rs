#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use serde_json::json;

use super::{selected_cached_cli_sso_team_detail, selected_cli_sso_team_detail};
use crate::cli_team_grants::records::{CliSsoTeamDetail, TEAMLESS_CLI_SSO_TEAM_DETAIL};

fn detail(team_id: Option<&str>, models: &[&str]) -> CliSsoTeamDetail {
    CliSsoTeamDetail {
        team_id: team_id.map(String::from),
        team_alias: None,
        team_models: models.iter().map(|model| String::from(*model)).collect(),
        team_model_aliases: None,
    }
}

#[test]
fn no_team_id_selects_the_teamless_detail() {
    let details = vec![detail(Some("team-1"), &["gpt-4"])];
    assert_eq!(
        selected_cli_sso_team_detail(&details, None),
        Some(TEAMLESS_CLI_SSO_TEAM_DETAIL)
    );
}

#[test]
fn an_empty_team_id_matches_a_detail_with_an_empty_id() {
    let details = vec![detail(Some(""), &[])];
    assert_eq!(
        selected_cli_sso_team_detail(&details, Some("")),
        Some(detail(Some(""), &[]))
    );
}

#[test]
fn selects_the_first_detail_with_the_team_id() {
    let details = vec![
        detail(Some("team-1"), &["first"]),
        detail(Some("team-1"), &["second"]),
    ];
    assert_eq!(
        selected_cli_sso_team_detail(&details, Some("team-1"))
            .unwrap()
            .team_models,
        vec![String::from("first")]
    );
}

#[test]
fn an_unknown_team_id_gives_none() {
    let details = vec![detail(Some("team-1"), &[])];
    assert_eq!(selected_cli_sso_team_detail(&details, Some("team-2")), None);
}

#[test]
fn a_detail_without_a_team_id_never_matches() {
    let details = vec![detail(None, &[])];
    assert_eq!(selected_cli_sso_team_detail(&details, Some("team-1")), None);
}

#[test]
fn no_team_id_ignores_a_corrupt_cache() {
    let cached = json!("not a list");
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&cached), None),
        Some(TEAMLESS_CLI_SSO_TEAM_DETAIL)
    );
    assert_eq!(
        selected_cached_cli_sso_team_detail(None, None),
        Some(TEAMLESS_CLI_SSO_TEAM_DETAIL)
    );
}

#[test]
fn a_missing_or_null_cache_is_unknown() {
    assert_eq!(
        selected_cached_cli_sso_team_detail(None, Some("team-1")),
        None
    );
    let null = json!(null);
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&null), Some("team-1")),
        None
    );
}

#[rstest]
#[case(json!("text"))]
#[case(json!(1))]
#[case(json!(true))]
#[case(json!({"team_id": "team-1", "team_models": []}))]
fn a_cached_value_that_is_not_a_list_is_unknown(#[case] cached: JsonValue) {
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&cached), Some("team-1")),
        None
    );
}

#[rstest]
#[case(json!([{"team_id": "team-1"}]))]
#[case(json!([{"team_id": "team-1", "team_models": [1]}]))]
#[case(json!([["team-1"]]))]
#[case(json!([{"team_id": "team-1", "team_models": [], "team_model_aliases": {"gpt-4": 1}}]))]
fn a_cached_entry_that_does_not_decode_is_unknown(#[case] cached: JsonValue) {
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&cached), Some("team-1")),
        None
    );
}

#[test]
fn reads_a_cached_list_and_ignores_unknown_keys() {
    let cached = json!([
        {
            "team_id": "team-1",
            "team_alias": "One",
            "team_models": ["gpt-4"],
            "team_model_aliases": {"gpt-4": "gpt-4o"},
            "unknown": 1
        },
        {"team_id": "team-2", "team_models": []}
    ]);
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&cached), Some("team-1")),
        Some(CliSsoTeamDetail {
            team_id: Some(String::from("team-1")),
            team_alias: Some(String::from("One")),
            team_models: vec![String::from("gpt-4")],
            team_model_aliases: Some(BTreeMap::from([(
                String::from("gpt-4"),
                String::from("gpt-4o")
            )])),
        })
    );
}

#[test]
fn a_cached_list_without_the_team_id_gives_none() {
    let cached = json!([{"team_id": "team-2", "team_models": []}]);
    assert_eq!(
        selected_cached_cli_sso_team_detail(Some(&cached), Some("team-1")),
        None
    );
}
