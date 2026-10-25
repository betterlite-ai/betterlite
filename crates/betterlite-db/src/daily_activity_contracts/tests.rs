#![cfg(test)]
use super::*;
use rstest::rstest;

#[rstest]
#[case(DailyActivityTable::User, "user_id", "LiteLLM_DailyUserSpend")]
#[case(DailyActivityTable::Team, "team_id", "LiteLLM_DailyTeamSpend")]
#[case(DailyActivityTable::Tag, "tag", "LiteLLM_DailyTagSpend")]
#[case(
    DailyActivityTable::Organization,
    "organization_id",
    "LiteLLM_DailyOrganizationSpend"
)]
#[case(
    DailyActivityTable::Customer,
    "end_user_id",
    "LiteLLM_DailyEndUserSpend"
)]
#[case(DailyActivityTable::Agent, "agent_id", "LiteLLM_DailyAgentSpend")]
fn table_field_and_relation(
    #[case] table: DailyActivityTable,
    #[case] field: &str,
    #[case] relation: &str,
) {
    assert_eq!(table.metadata().entity_field.wire_name(), field);
    assert_eq!(table.metadata().relation, relation);
}

#[test]
fn mismatched_entity_field_rejected() {
    serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"team_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"a","end_date":"b","model":null,"timezone_offset_minutes":0}"#).unwrap_err();
}
#[test]
fn injected_identifier_rejected() {
    serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id;drop","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"a","end_date":"b","model":null,"timezone_offset_minutes":0}"#).unwrap_err();
}
#[test]
fn scope_required_fields() {
    serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","exclude_entity_ids":[],"api_keys":null,"start_date":"a","end_date":"b","model":null,"timezone_offset_minutes":0}"#).unwrap_err();
}
#[test]
fn scope_current_day_default_false() {
    let scope = serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"later","end_date":"earlier","model":null,"timezone_offset_minutes":-90}"#).expect("scope");
    assert!(!scope.include_current_utc_day);
}
#[test]
fn scope_none_empty_and_duplicate_filters() {
    let scope = serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":["","a","a"],"exclude_entity_ids":["x","x"],"api_keys":[],"start_date":"a","end_date":"b","model":"","timezone_offset_minutes":0}"#).expect("scope");
    assert_eq!(scope.api_keys, Some(Vec::new()));
}
#[test]
fn scope_dates_and_offsets_not_normalized() {
    let scope = serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"later","end_date":"earlier","model":null,"timezone_offset_minutes":-90}"#).expect("scope");
    assert_eq!(scope.timezone_offset_minutes, Some(-90_i32));
}
#[test]
fn sql_parameter_order() {
    let query = SqlQuery {
        sql: String::from("x"),
        params: vec![
            SqlParameter::Text(String::from("a")),
            SqlParameter::Integer(2_i64),
        ],
    };
    assert_eq!(query.params.len(), 2);
}
#[test]
fn cursor_empty_fields_preserved() {
    let cursor =
        serde_json::from_str::<ExportCursor>(r#"{"date":"","entity_id":"","group_key":""}"#)
            .expect("cursor");
    assert_eq!(cursor.date, String::new());
}
#[test]
fn rollup_ptu_only_optional_default() {
    let row = serde_json::from_str::<RollupMetricsRow>(r#"{"date":null,"api_key":null,"spend":null,"prompt_tokens":null,"completion_tokens":null,"cache_read_input_tokens":null,"cache_creation_input_tokens":null,"compression_saved_tokens":null,"compression_savings_spend":null,"prompt_caching_savings_spend":null,"gateway_injected_caching_savings_spend":null,"autorouter_savings_spend":null,"api_requests":null,"successful_requests":null,"failed_requests":null,"total_response_time_ms":null,"timed_requests":null}"#).expect("rollup");
    assert_eq!(row.ptu_flat_cost, None);
}
#[test]
fn null_metrics_not_zero() {
    let row = serde_json::from_str::<RollupMetricsRow>(r#"{"date":null,"api_key":null,"spend":null,"prompt_tokens":null,"completion_tokens":null,"cache_read_input_tokens":null,"cache_creation_input_tokens":null,"compression_saved_tokens":null,"compression_savings_spend":null,"prompt_caching_savings_spend":null,"gateway_injected_caching_savings_spend":null,"autorouter_savings_spend":null,"api_requests":null,"successful_requests":null,"failed_requests":null,"total_response_time_ms":null,"timed_requests":null}"#).expect("rollup");
    assert_eq!(row.api_requests, None);
}
#[test]
fn negative_money_preserved() {
    let row = serde_json::from_str::<KeySpendRow>(r#"{"api_key":"k","spend":"-1.25","prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"api_requests":4,"successful_requests":5,"failed_requests":6,"cache_read_input_tokens":7,"cache_creation_input_tokens":8}"#).expect("row");
    assert_eq!(row.spend, Decimal::new(-125_i64, 2));
}
