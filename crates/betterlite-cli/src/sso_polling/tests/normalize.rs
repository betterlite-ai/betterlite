use serde_json::value::RawValue;

use crate::sso_polling::teams::normalize_teams;

fn details(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).expect("raw json")
}

#[test]
fn normalize_reads_ids_and_aliases_from_team_details() {
    let team_details = details(r#"[{"team_id":"team-1","team_alias":"Alpha"},{"id":"team-2"}]"#);

    let teams = normalize_teams(&[String::from("ignored")], Some(team_details.as_ref()))
        .expect("the teams");

    assert_eq!(teams.len(), 2);
    assert_eq!(teams[0].team_id.as_deref(), Some("team-1"));
    assert_eq!(teams[0].team_alias.as_deref(), Some("Alpha"));
    assert!(teams[0].models.is_empty());
    assert!(teams[0].max_budget.is_none());
    assert_eq!(teams[1].team_id.as_deref(), Some("team-2"));
    assert_eq!(teams[1].team_alias, None);
}

#[test]
fn normalize_prefers_the_team_id_over_the_id() {
    let team_details = details(r#"[{"team_id":"team-1","id":"team-9"}]"#);

    let teams = normalize_teams(&[], Some(team_details.as_ref())).expect("the teams");

    assert_eq!(teams[0].team_id.as_deref(), Some("team-1"));
}

#[test]
fn normalize_falls_back_to_the_team_ids_without_details() {
    let teams = normalize_teams(&[String::from("team-1"), String::from("team-2")], None)
        .expect("the teams");

    assert_eq!(teams.len(), 2);
    assert_eq!(teams[0].team_id.as_deref(), Some("team-1"));
    assert_eq!(teams[0].team_alias, None);
    assert_eq!(teams[1].team_id.as_deref(), Some("team-2"));
}

#[test]
fn normalize_falls_back_when_the_details_list_is_empty() {
    let team_details = details("[]");

    let teams =
        normalize_teams(&[String::from("team-1")], Some(team_details.as_ref())).expect("the teams");

    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].team_id.as_deref(), Some("team-1"));
}

#[test]
fn normalize_keeps_the_details_and_skips_entries_without_an_id() {
    let team_details = details(r#"[{"team_alias":"Alpha"}]"#);

    let teams =
        normalize_teams(&[String::from("team-1")], Some(team_details.as_ref())).expect("the teams");

    assert!(teams.is_empty());
}

#[test]
fn normalize_ignores_detail_models_and_budget() {
    let team_details = details(r#"[{"team_id":"team-1","models":["gpt-4o"],"max_budget":5}]"#);

    let teams = normalize_teams(&[], Some(team_details.as_ref())).expect("the teams");

    assert!(teams[0].models.is_empty());
    assert!(teams[0].max_budget.is_none());
}

#[test]
fn normalize_rejects_details_that_are_not_a_list() {
    let team_details = details(r#"{"team_id":"team-1"}"#);

    let error = normalize_teams(&[String::from("team-1")], Some(team_details.as_ref()))
        .expect_err("the details");

    assert!(error.to_string().contains("team details"));
}

#[test]
fn normalize_names_a_wrongly_typed_detail_field() {
    let team_details = details(r#"[{"team_id":5}]"#);

    let error = normalize_teams(&[], Some(team_details.as_ref())).expect_err("the details");

    assert!(error.to_string().contains("team_id"));
}

#[test]
fn normalize_returns_no_teams_without_details_or_ids() {
    let teams = normalize_teams(&[], None).expect("the teams");

    assert!(teams.is_empty());
}
