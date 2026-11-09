use rstest::rstest;

use super::support::{expected, row};

const IDENTITY_PAYLOAD: &str = r#"{
    "metadata": {
        "user_api_key_hash": "hash-1",
        "user_api_key_alias": "alias-1",
        "user_api_key_team_id": "team-1",
        "user_api_key_team_alias": "team-alias-1",
        "user_api_key_user_id": "user-1",
        "user_api_key_org_id": "org-1"
    }
}"#;

#[test]
fn the_key_and_team_identity_is_lifted_out_of_metadata() {
    let row = row(IDENTITY_PAYLOAD);
    assert_eq!(row.api_key_hash.as_deref(), Some("hash-1"));
    assert_eq!(row.api_key_alias.as_deref(), Some("alias-1"));
    assert_eq!(row.team_id.as_deref(), Some("team-1"));
    assert_eq!(row.team_alias.as_deref(), Some("team-alias-1"));
    assert_eq!(row.user_id.as_deref(), Some("user-1"));
    assert_eq!(row.org_id.as_deref(), Some("org-1"));
}

#[test]
fn the_metadata_column_holds_the_whole_object_as_json_text() {
    let row = row(IDENTITY_PAYLOAD);
    let text = row.metadata.unwrap();
    assert_eq!(
        expected(&text),
        expected(
            r#"{
                "user_api_key_hash": "hash-1",
                "user_api_key_alias": "alias-1",
                "user_api_key_team_id": "team-1",
                "user_api_key_team_alias": "team-alias-1",
                "user_api_key_user_id": "user-1",
                "user_api_key_org_id": "org-1"
            }"#
        )
    );
}

#[test]
fn metadata_that_is_not_an_object_projects_no_scalar_columns() {
    let row = row(r#"{"metadata": "later"}"#);
    assert_eq!(row.api_key_hash, None);
    assert_eq!(row.api_key_alias, None);
    assert_eq!(row.team_id, None);
    assert_eq!(row.team_alias, None);
    assert_eq!(row.user_id, None);
    assert_eq!(row.org_id, None);
    assert!(matches!(row.metadata, Some(text) if text == "\"later\""));
}

#[test]
fn missing_metadata_projects_no_scalar_columns() {
    let row = row("{}");
    assert_eq!(row.api_key_hash, None);
    assert_eq!(row.team_id, None);
    assert_eq!(row.metadata, None);
}

#[test]
fn an_empty_metadata_object_projects_no_scalar_columns() {
    let row = row(r#"{"metadata": {}}"#);
    assert_eq!(row.api_key_hash, None);
    assert_eq!(row.team_id, None);
    assert_eq!(row.metadata.as_deref(), Some("{}"));
}

#[rstest]
#[case::number("5")]
#[case::boolean("true")]
#[case::object(r#"{"nested": "x"}"#)]
fn a_metadata_value_that_is_not_text_is_null(#[case] value: &str) {
    let row = row(&format!(
        r#"{{"metadata": {{"user_api_key_hash": {value}}}}}"#
    ));
    assert_eq!(row.api_key_hash, None);
}
