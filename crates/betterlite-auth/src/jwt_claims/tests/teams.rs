use rstest::rstest;

use crate::jwt_claims::claims::ClaimError;
use crate::jwt_claims::settings::JwtAuthSettings;
use crate::jwt_claims::teams::TeamClaims as _;

use super::support::{empty_settings, issuers, project};

fn team_settings() -> JwtAuthSettings {
    let mut settings = empty_settings();
    settings.team_id_jwt_field = Some("team".to_owned());
    settings.team_ids_jwt_field = Some("groups".to_owned());
    settings.team_id_default = Some("default-team".to_owned());
    settings.team_alias_jwt_field = Some("team.name".to_owned());
    settings
}

fn marker_settings() -> JwtAuthSettings {
    let mut settings = empty_settings();
    settings.team_id_jwt_field = Some("team".to_owned());
    settings.team_ids_jwt_field = Some("groups".to_owned());
    settings
}

#[test]
fn team_array() {
    let settings = team_settings();
    let claims = project(
        r#"{"groups":["team-a","team-b"],"team":["team-c","team-a"]}"#,
        &settings,
        &[],
    );
    assert_eq!(claims.team_ids().unwrap(), vec!["team-a", "team-b"]);
    assert_eq!(
        claims.all_team_ids().unwrap(),
        vec!["team-a", "team-b", "team-c"]
    );
    assert_eq!(claims.team_id(None).unwrap(), Some("team-c".to_owned()));
}

#[test]
fn team_array_deduplicates_and_skips_empty_values() {
    let settings = team_settings();
    let duplicates = project(
        r#"{"groups":["team-a"],"team":["team-a","team-a","team-b","team-b"]}"#,
        &settings,
        &[],
    );
    assert_eq!(duplicates.all_team_ids().unwrap(), vec!["team-a", "team-b"]);
    let nulls = project(
        r#"{"groups":["team-a"],"team":[null,"","team-b",null]}"#,
        &settings,
        &[],
    );
    assert_eq!(nulls.all_team_ids().unwrap(), vec!["team-a", "team-b"]);
    let scalar = project(r#"{"groups":["team-a"],"team":"team-a"}"#, &settings, &[]);
    assert_eq!(scalar.all_team_ids().unwrap(), vec!["team-a"]);
    let empty_scalar = project(r#"{"groups":["team-a"],"team":""}"#, &settings, &[]);
    assert_eq!(empty_scalar.all_team_ids().unwrap(), vec!["team-a"]);
}

#[test]
fn team_array_from_the_issuer_marker() {
    let settings = marker_settings();
    let trusted_issuers = issuers(&["https://idp.example"]);
    let claims = project(
        r#"{
            "_litellm_jwt_issuer": "https://idp.example",
            "_litellm_team_ids": ["trusted-a", "trusted-b"],
            "_litellm_team_id": ["trusted-c", "trusted-a"],
            "groups": ["raw-a"],
            "team": "raw-team"
        }"#,
        &settings,
        &trusted_issuers,
    );
    assert_eq!(claims.team_ids().unwrap(), vec!["trusted-a", "trusted-b"]);
    assert_eq!(
        claims.all_team_ids().unwrap(),
        vec!["trusted-a", "trusted-b", "trusted-c"]
    );
    assert_eq!(claims.team_id(None).unwrap(), Some("trusted-c".to_owned()));
}

#[test]
fn marker_team_ids_accept_a_string_and_reject_other_scalars() {
    let settings = JwtAuthSettings::default();
    let trusted_issuers = issuers(&["https://idp.example"]);
    let string = project(
        r#"{"_litellm_jwt_issuer": "https://idp.example", "_litellm_team_ids": "trusted-a", "_litellm_team_id": "trusted-b"}"#,
        &settings,
        &trusted_issuers,
    );
    assert_eq!(string.team_ids().unwrap(), vec!["trusted-a"]);
    assert_eq!(string.team_id(None).unwrap(), Some("trusted-b".to_owned()));
    let scalar = project(
        r#"{"_litellm_jwt_issuer": "https://idp.example", "_litellm_team_ids": 5, "_litellm_team_id": 7}"#,
        &settings,
        &trusted_issuers,
    );
    assert_eq!(scalar.team_ids().unwrap(), Vec::<String>::new());
}

#[test]
fn a_null_marker_claim_shadows_the_configured_field() {
    let settings = marker_settings();
    let trusted_issuers = issuers(&["https://idp.example"]);
    let claims = project(
        r#"{"_litellm_jwt_issuer": "https://idp.example", "_litellm_team_id": null, "team": "raw-team"}"#,
        &settings,
        &trusted_issuers,
    );
    assert_eq!(claims.team_id(None).unwrap(), None);
    assert_eq!(claims.all_team_ids().unwrap(), Vec::<String>::new());
}

#[rstest]
#[case::plural_string(r#"{"groups":"team-a"}"#, "groups")]
#[case::plural_number(r#"{"groups":5}"#, "groups")]
#[case::plural_null(r#"{"groups":null}"#, "groups")]
#[case::plural_missing("{}", "groups")]
fn plural_claims_that_are_not_lists_are_empty(#[case] claims: &str, #[case] field: &str) {
    let mut settings = empty_settings();
    settings.team_ids_jwt_field = Some(field.to_owned());
    let projected = project(claims, &settings, &[]);
    assert_eq!(projected.team_ids().unwrap(), Vec::<String>::new());
}

#[test]
fn team_id_sources() {
    let settings = team_settings();
    let nested = project(r#"{"team":{"id":"team-1"}}"#, &settings, &[]);
    assert_eq!(
        nested.team_id(Some("given")).unwrap(),
        Some("team-1".to_owned())
    );
    let list = project(r#"{"team":["team-a","team-b"]}"#, &settings, &[]);
    assert_eq!(list.team_id(None).unwrap(), Some("team-a".to_owned()));
    let null_first = project(r#"{"team":[null,"team-b"]}"#, &settings, &[]);
    assert_eq!(null_first.team_id(Some("given")).unwrap(), None);
    let null_singular = project(r#"{"team":null}"#, &settings, &[]);
    assert_eq!(null_singular.team_id(Some("given")).unwrap(), None);
}

#[test]
fn team_id_defaults() {
    let settings = team_settings();
    let empty_list = project(r#"{"team":[]}"#, &settings, &[]);
    assert_eq!(
        empty_list.team_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    let absent = project("{}", &settings, &[]);
    assert_eq!(
        absent.team_id(Some("given")).unwrap(),
        Some("default-team".to_owned())
    );

    let mut configured_only = empty_settings();
    configured_only.team_id_default = Some("default-team".to_owned());
    let no_field = project("{}", &configured_only, &[]);
    assert_eq!(
        no_field.team_id(Some("given")).unwrap(),
        Some("default-team".to_owned())
    );

    let no_settings = JwtAuthSettings::default();
    let unset_field = project(r#"{"team":"team-1"}"#, &no_settings, &[]);
    assert_eq!(unset_field.team_id(Some("given")).unwrap(), None);
}

#[test]
fn marker_team_id_defaults() {
    let settings = JwtAuthSettings::default();
    let trusted_issuers = issuers(&["https://idp.example"]);
    let empty = project(
        r#"{"_litellm_jwt_issuer": "https://idp.example", "_litellm_team_id": []}"#,
        &settings,
        &trusted_issuers,
    );
    assert_eq!(
        empty.team_id(Some("given")).unwrap(),
        Some("given".to_owned())
    );
    let number = project(
        r#"{"_litellm_jwt_issuer": "https://idp.example", "_litellm_team_id": 5}"#,
        &settings,
        &trusted_issuers,
    );
    assert!(matches!(
        number.team_id(None),
        Err(ClaimError::Type { claim, .. }) if claim == "_litellm_team_id"
    ));
}

#[test]
fn team_alias() {
    let settings = team_settings();
    let found = project(r#"{"team":{"name":"core"}}"#, &settings, &[]);
    assert_eq!(found.team_alias(None).unwrap(), Some("core".to_owned()));
    let missing = project(r#"{"team":{}}"#, &settings, &[]);
    assert_eq!(
        missing.team_alias(Some("fallback")).unwrap(),
        Some("fallback".to_owned())
    );
    let null = project(r#"{"team":{"name":null}}"#, &settings, &[]);
    assert_eq!(
        null.team_alias(Some("fallback")).unwrap(),
        Some("fallback".to_owned())
    );
}

#[test]
fn team_alias_rejects_a_non_string_and_an_unset_field() {
    let settings = team_settings();
    let number = project(r#"{"team":{"name":5}}"#, &settings, &[]);
    assert!(matches!(
        number.team_alias(None),
        Err(ClaimError::Type { claim, .. }) if claim == "team.name"
    ));
    let default_settings = JwtAuthSettings::default();
    let unset = project(r#"{"team":{"name":"core"}}"#, &default_settings, &[]);
    assert_eq!(unset.team_alias(Some("fallback")).unwrap(), None);
}

#[rstest]
#[case::both_fields(Some("team"), Some("alias"), true)]
#[case::id_field(Some("team"), None, true)]
#[case::alias_field(None, Some("alias"), true)]
#[case::empty_values(Some(""), Some(""), true)]
#[case::neither(None, None, false)]
fn required_team_id(
    #[case] team_id_jwt_field: Option<&str>,
    #[case] team_alias_jwt_field: Option<&str>,
    #[case] expected: bool,
) {
    let mut settings = empty_settings();
    settings.team_id_jwt_field = team_id_jwt_field.map(str::to_owned);
    settings.team_alias_jwt_field = team_alias_jwt_field.map(str::to_owned);
    let projected = project("{}", &settings, &[]);
    assert_eq!(
        projected.settings.team_id_jwt_field.is_some()
            || projected.settings.team_alias_jwt_field.is_some(),
        expected
    );
}
