use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::tags::extract_tags;

fn tags(metadata: Json) -> IndexMap<String, String> {
    let settings = support::settings(&support::init(&[]));
    let payload = support::default_payload()
        .set("/metadata", metadata)
        .build();
    extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        payload.as_ref(),
    )
}

#[test]
fn user_team_tags_are_normalized_for_the_alias_and_team_alias() {
    let found = tags(support::object(vec![
        ("user_api_key_alias", support::text("Alice & Co")),
        ("user_api_key_team_alias", support::text("Team One")),
    ]));
    assert_eq!(support::tag(&found, "user"), Some("alice_co"));
    assert_eq!(support::tag(&found, "team"), Some("team_one"));
}

#[rstest]
#[case::team_id_only(vec![("team_id", "Team ID")], "team_id")]
#[case::team_alias_beats_team_id(
    vec![("team_alias", "Team Alias"), ("team_id", "Team ID")],
    "team_alias"
)]
#[case::team_alias_beats_api_key_team_id(
    vec![("team_alias", "Team Alias"), ("user_api_key_team_id", "Key Team")],
    "team_alias"
)]
#[case::api_key_team_id_beats_team_id(
    vec![("user_api_key_team_id", "Key Team"), ("team_id", "Team ID")],
    "key_team"
)]
#[case::api_key_team_alias_beats_all(
    vec![
        ("user_api_key_team_alias", "Key Team Alias"),
        ("user_api_key_team_id", "Key Team"),
        ("team_alias", "Team Alias"),
        ("team_id", "Team ID"),
    ],
    "key_team_alias"
)]
fn user_team_precedence_follows_the_metadata_chain(
    #[case] metadata: Vec<(&str, &str)>,
    #[case] expected: &str,
) {
    let entries = metadata
        .into_iter()
        .map(|(key, value)| (key, support::text(value)))
        .collect();
    assert_eq!(
        support::tag(&tags(support::object(entries)), "team"),
        Some(expected)
    );
}

#[test]
fn user_team_model_group_comes_from_the_payload_field() {
    let found = tags(support::object(Vec::new()));
    assert_eq!(support::tag(&found, "model_group"), Some("production"));
}

#[test]
fn user_team_tags_are_absent_without_metadata() {
    let found = tags(support::object(Vec::new()));
    assert_eq!(support::tag(&found, "user"), None);
    assert_eq!(support::tag(&found, "team"), None);
}
