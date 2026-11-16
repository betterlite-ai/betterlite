#![cfg(test)]

use betterlite_db::daily_activity_contracts::identifiers::{
    DailyActivityError, DailyActivityTable,
};
use rstest::rstest;

use super::super::scope::{ScopeIds, ScopeRequest, daily_activity_scope};

fn request(table: DailyActivityTable, entity_id_field: &str) -> ScopeRequest<'_> {
    ScopeRequest {
        table,
        entity_id_field,
        entity_id: None,
        exclude_entity_ids: Vec::new(),
        api_key: None,
        start_date: "2026-01-01",
        end_date: "2026-01-31",
        model: None,
        timezone_offset_minutes: None,
        include_current_utc_day: false,
    }
}

#[test]
fn scope_leaves_entity_ids_none_without_a_filter() {
    let scope = daily_activity_scope(request(DailyActivityTable::User, "user_id")).unwrap();
    let input = scope.as_ref();
    assert!(input.entity_ids.is_none());
    assert!(input.api_keys.is_none());
    assert!(input.exclude_entity_ids.is_empty());
    assert!(!input.include_current_utc_day);
}

#[test]
fn scope_keeps_a_single_entity_id() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.entity_id = Some(ScopeIds::One("user-1".to_owned()));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(scope.as_ref().entity_ids, Some(vec!["user-1".to_owned()]));
}

#[test]
fn scope_keeps_an_entity_id_list_in_order() {
    let mut call = request(DailyActivityTable::Team, "team_id");
    call.entity_id = Some(ScopeIds::Many(vec![
        "team-b".to_owned(),
        "team-a".to_owned(),
    ]));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(
        scope.as_ref().entity_ids,
        Some(vec!["team-b".to_owned(), "team-a".to_owned()])
    );
}

#[test]
fn scope_keeps_an_empty_entity_id_list() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.entity_id = Some(ScopeIds::Many(Vec::new()));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(scope.as_ref().entity_ids, Some(Vec::new()));
}

#[test]
fn scope_turns_an_empty_api_key_into_no_filter() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.api_key = Some(ScopeIds::One(String::new()));
    let scope = daily_activity_scope(call).unwrap();
    assert!(scope.as_ref().api_keys.is_none());
}

#[test]
fn scope_keeps_a_single_api_key() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.api_key = Some(ScopeIds::One("sk-one".to_owned()));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(scope.as_ref().api_keys, Some(vec!["sk-one".to_owned()]));
}

#[test]
fn scope_keeps_an_api_key_list() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.api_key = Some(ScopeIds::Many(vec![
        "sk-one".to_owned(),
        "sk-two".to_owned(),
    ]));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(
        scope.as_ref().api_keys,
        Some(vec!["sk-one".to_owned(), "sk-two".to_owned()])
    );
}

#[test]
fn scope_keeps_an_empty_api_key_list_as_an_empty_filter() {
    let mut call = request(DailyActivityTable::User, "user_id");
    call.api_key = Some(ScopeIds::Many(Vec::new()));
    let scope = daily_activity_scope(call).unwrap();
    assert_eq!(scope.as_ref().api_keys, Some(Vec::new()));
}

#[test]
fn scope_passes_every_remaining_parameter_through() {
    let mut call = request(DailyActivityTable::Team, "team_id");
    call.entity_id = Some(ScopeIds::One("team-1".to_owned()));
    call.exclude_entity_ids = vec!["team-skip".to_owned()];
    call.start_date = "2026-02-01";
    call.end_date = "2026-02-28";
    call.model = Some("gpt-4o");
    call.timezone_offset_minutes = Some(-330);
    call.include_current_utc_day = true;
    let scope = daily_activity_scope(call).unwrap();
    let input = scope.as_ref();
    assert_eq!(input.table, DailyActivityTable::Team);
    assert_eq!(input.entity_id_field, "team_id");
    assert_eq!(input.start_date, "2026-02-01");
    assert_eq!(input.end_date, "2026-02-28");
    assert_eq!(input.model.as_deref(), Some("gpt-4o"));
    assert_eq!(input.timezone_offset_minutes, Some(-330));
    assert!(input.include_current_utc_day);
    assert_eq!(input.exclude_entity_ids, vec!["team-skip".to_owned()]);
}

#[test]
fn scope_defaults_excluded_entities_to_none_requested() {
    let scope = daily_activity_scope(request(DailyActivityTable::User, "user_id")).unwrap();
    assert!(scope.as_ref().exclude_entity_ids.is_empty());
}

#[test]
fn scope_rejects_an_entity_field_from_another_table() {
    let error = daily_activity_scope(request(DailyActivityTable::User, "team_id")).unwrap_err();
    assert_eq!(
        error,
        DailyActivityError::EntityField {
            field: "team_id".to_owned(),
            table: "litellm_dailyuserspend",
        }
    );
}

#[rstest]
#[case(DailyActivityTable::User, "user_id")]
#[case(DailyActivityTable::Team, "team_id")]
#[case(DailyActivityTable::Tag, "tag")]
#[case(DailyActivityTable::Organization, "organization_id")]
#[case(DailyActivityTable::Customer, "end_user_id")]
#[case(DailyActivityTable::Agent, "agent_id")]
fn scope_accepts_each_tables_entity_field(#[case] table: DailyActivityTable, #[case] field: &str) {
    let scope = daily_activity_scope(request(table, field)).unwrap();
    assert_eq!(scope.as_ref().table, table);
    assert_eq!(scope.as_ref().entity_id_field, field);
}
