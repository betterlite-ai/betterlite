#![cfg(test)]

use super::WindowSource;

#[test]
fn window_sources_match_the_litellm_table_definitions() {
    let keys = WindowSource::Keys;
    assert_eq!(keys.table(), "LiteLLM_VerificationToken");
    assert_eq!(keys.id_column(), "token");
    assert_eq!(keys.entity_type(), "key");
    assert_eq!(keys.counter_prefix(), "spend:key");
    assert_eq!(keys.log_subject(), "keys");
    let teams = WindowSource::Teams;
    assert_eq!(teams.table(), "LiteLLM_TeamTable");
    assert_eq!(teams.id_column(), "team_id");
    assert_eq!(teams.entity_type(), "team");
    assert_eq!(teams.counter_prefix(), "spend:team");
    assert_eq!(teams.log_subject(), "teams");
}

#[test]
fn window_sources_walk_keys_before_teams() {
    assert_eq!(WindowSource::ALL, [WindowSource::Keys, WindowSource::Teams]);
}

#[test]
fn window_source_retry_reasons_name_the_entity() {
    assert_eq!(
        WindowSource::Keys.read_reason(),
        "reset_budget_read_key_windows_failure"
    );
    assert_eq!(
        WindowSource::Keys.write_reason(),
        "reset_budget_write_key_windows_failure"
    );
    assert_eq!(
        WindowSource::Teams.read_reason(),
        "reset_budget_read_team_windows_failure"
    );
    assert_eq!(
        WindowSource::Teams.write_reason(),
        "reset_budget_write_team_windows_failure"
    );
}

#[test]
fn window_source_page_queries_keyset_on_the_primary_key() {
    let sql = WindowSource::Teams.page_query();
    assert!(sql.contains("FROM \"LiteLLM_TeamTable\""));
    assert!(sql.contains("budget_limits IS NOT NULL"));
    assert!(sql.contains("team_id > $1"));
    assert!(sql.contains("ORDER BY team_id LIMIT $2"));
    let keys = WindowSource::Keys.page_query();
    assert!(keys.contains("FROM \"LiteLLM_VerificationToken\""));
    assert!(keys.contains("token > $1"));
}

#[test]
fn window_source_update_stores_the_list_as_a_json_string() {
    assert_eq!(
        WindowSource::Keys.update_query(),
        "UPDATE \"LiteLLM_VerificationToken\" SET budget_limits = to_jsonb($2::text) WHERE token = $1"
    );
    assert_eq!(
        WindowSource::Teams.update_query(),
        "UPDATE \"LiteLLM_TeamTable\" SET budget_limits = to_jsonb($2::text) WHERE team_id = $1"
    );
}

#[test]
fn window_counter_keys_name_the_entity_and_the_window() {
    assert_eq!(
        WindowSource::Keys.window_counter_key("sk-1", "1h"),
        "spend:key:sk-1:window:1h"
    );
    assert_eq!(
        WindowSource::Teams.window_counter_key("team-1", "1d"),
        "spend:team:team-1:window:1d"
    );
}
