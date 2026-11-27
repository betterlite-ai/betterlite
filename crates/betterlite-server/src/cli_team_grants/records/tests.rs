#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::identity_grants::team_grants::TeamColumnsError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::aliases::ModelAliases;
use rstest::rstest;

use super::{CliSsoTeamDetail, TEAMLESS_CLI_SSO_TEAM_DETAIL, TeamGrantRow, cli_sso_team_detail};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(alias, model)| (String::from(*alias), String::from(*model)))
        .collect()
}

fn aliases(entries: &[(&str, &str)]) -> ModelAliases {
    ModelAliases::Map(
        entries
            .iter()
            .map(|(alias, model)| (String::from(*alias), JsonData::String(String::from(*model))))
            .collect(),
    )
}

fn detail_for(model_aliases: Option<ModelAliases>) -> CliSsoTeamDetail {
    cli_sso_team_detail(TeamGrantRow {
        team_id: String::from("team-1"),
        team_alias: Some(String::from("Team One")),
        models: strings(&["gpt-4", "claude"]),
        model_aliases,
    })
    .expect("the row projects")
}

#[test]
fn decodes_a_detail_that_only_names_its_models() {
    let detail: CliSsoTeamDetail = serde_json::from_str(r#"{"team_models": ["gpt-4"]}"#).unwrap();
    assert_eq!(
        detail,
        CliSsoTeamDetail {
            team_id: None,
            team_alias: None,
            team_models: strings(&["gpt-4"]),
            team_model_aliases: None,
        }
    );
}

#[test]
fn refuses_a_detail_without_team_models() {
    let error = serde_json::from_str::<CliSsoTeamDetail>(r#"{"team_id": "team-1"}"#)
        .expect_err("team_models is required");
    assert!(error.to_string().contains("team_models"));
}

#[test]
fn reads_explicit_nulls_as_absent() {
    let detail: CliSsoTeamDetail = serde_json::from_str(
        r#"{"team_id": null, "team_alias": null, "team_models": [], "team_model_aliases": null}"#,
    )
    .unwrap();
    assert_eq!(
        detail,
        CliSsoTeamDetail {
            team_id: None,
            team_alias: None,
            team_models: Vec::new(),
            team_model_aliases: None,
        }
    );
}

#[test]
fn ignores_unknown_keys() {
    let detail: CliSsoTeamDetail =
        serde_json::from_str(r#"{"team_models": ["gpt-4"], "unknown": {"nested": true}}"#).unwrap();
    assert_eq!(detail.team_models, strings(&["gpt-4"]));
}

#[test]
fn serializes_all_four_keys_with_null_for_absent_values() {
    let value = serde_json::to_value(CliSsoTeamDetail {
        team_id: Some(String::from("team-1")),
        team_alias: None,
        team_models: strings(&["gpt-4"]),
        team_model_aliases: None,
    })
    .unwrap();
    let expected: JsonValue = serde_json::from_str(
        r#"{"team_id": "team-1", "team_alias": null, "team_models": ["gpt-4"], "team_model_aliases": null}"#,
    )
    .unwrap();
    assert_eq!(value, expected);
}

#[test]
fn the_teamless_detail_restricts_nothing() {
    assert_eq!(
        TEAMLESS_CLI_SSO_TEAM_DETAIL,
        CliSsoTeamDetail {
            team_id: None,
            team_alias: None,
            team_models: Vec::new(),
            team_model_aliases: None,
        }
    );
}

#[test]
fn projects_a_row_without_an_alias_table() {
    assert_eq!(
        detail_for(None),
        CliSsoTeamDetail {
            team_id: Some(String::from("team-1")),
            team_alias: Some(String::from("Team One")),
            team_models: strings(&["gpt-4", "claude"]),
            team_model_aliases: None,
        }
    );
}

#[rstest]
#[case(r#"{"gpt-4": "gpt-4o"}"#, Some(map(&[("gpt-4", "gpt-4o")])))]
#[case(r#"{"gpt-4": 1}"#, None)]
#[case("not json", None)]
#[case("[]", None)]
#[case(r#""a string""#, None)]
fn reads_alias_text_as_a_map_or_no_aliases(
    #[case] text: &str,
    #[case] expected: Option<BTreeMap<String, String>>,
) {
    assert_eq!(
        detail_for(Some(ModelAliases::Text(String::from(text)))).team_model_aliases,
        expected
    );
}

#[test]
fn keeps_a_stored_map_of_text_aliases() {
    assert_eq!(
        detail_for(Some(aliases(&[("gpt-4", "gpt-4o")]))).team_model_aliases,
        Some(map(&[("gpt-4", "gpt-4o")]))
    );
}

#[test]
fn refuses_a_stored_alias_map_with_a_number() {
    let model_aliases: ModelAliases = serde_json::from_str(r#"{"gpt-4": 1}"#).unwrap();
    let error = cli_sso_team_detail(TeamGrantRow {
        team_id: String::from("team-1"),
        team_alias: None,
        models: Vec::new(),
        model_aliases: Some(model_aliases),
    })
    .expect_err("a value that is not text refuses the row");
    let TeamColumnsError::AliasNotText { alias } = error;
    assert_eq!(alias, "gpt-4");
}
