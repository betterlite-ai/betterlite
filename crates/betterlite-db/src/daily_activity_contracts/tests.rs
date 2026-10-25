#![cfg(test)]

use super::*;
use rstest::rstest;

#[rstest]
#[case::user(DailyActivityTable::User, "user_id", "LiteLLM_DailyUserSpend")]
#[case::team(DailyActivityTable::Team, "team_id", "LiteLLM_DailyTeamSpend")]
#[case::tag(DailyActivityTable::Tag, "tag", "LiteLLM_DailyTagSpend")]
#[case::organization(
    DailyActivityTable::Organization,
    "organization_id",
    "LiteLLM_DailyOrganizationSpend"
)]
#[case::customer(
    DailyActivityTable::Customer,
    "end_user_id",
    "LiteLLM_DailyEndUserSpend"
)]
#[case::agent(DailyActivityTable::Agent, "agent_id", "LiteLLM_DailyAgentSpend")]
fn table_field_and_relation(
    #[case] table: DailyActivityTable,
    #[case] field: &str,
    #[case] relation: &str,
) {
    assert_eq!(table.metadata().1, field);
    assert_eq!(table.metadata().2, relation);
}

#[test]
fn mismatched_entity_field_rejected() {
    let scope = DailyActivityScope {
        table: DailyActivityTable::User,
        entity_id_field: "team_id".to_owned(),
        entity_ids: None,
        exclude_entity_ids: Vec::new(),
        api_keys: None,
        start_date: "later".to_owned(),
        end_date: "earlier".to_owned(),
        model: None,
        timezone_offset_minutes: Some(-90_i32),
        include_current_utc_day: false,
    };
    assert_eq!(
        validate_scope(scope).unwrap_err(),
        "Invalid entity_id_field \"team_id\" for litellm_dailyuserspend"
    );
}

#[test]
fn scope_defaults_and_preserves_filters() {
    let scope = DailyActivityScope {
        table: DailyActivityTable::User,
        entity_id_field: "user_id".to_owned(),
        entity_ids: Some(vec![String::new(), "a".to_owned(), "a".to_owned()]),
        exclude_entity_ids: vec!["x".to_owned(), "x".to_owned()],
        api_keys: Some(Vec::new()),
        start_date: "z".to_owned(),
        end_date: "a".to_owned(),
        model: Some(String::new()),
        timezone_offset_minutes: Some(-30_i32),
        include_current_utc_day: false,
    };
    assert!(!scope.include_current_utc_day);
    assert_eq!(
        scope.entity_ids,
        Some(vec![String::new(), "a".to_owned(), "a".to_owned()])
    );
    assert_eq!(scope.api_keys, Some(Vec::new()));
}

#[rstest]
#[case::daily(ExportType::Daily, "daily")]
#[case::keys(ExportType::DailyWithKeys, "daily_with_keys")]
#[case::models(ExportType::DailyWithModels, "daily_with_models")]
#[case::users(ExportType::DailyWithUsers, "daily_with_users")]
fn export_type_wire_spellings(#[case] value: ExportType, #[case] wire: &str) {
    assert_eq!(serde_json::to_value(value).unwrap(), wire);
}

#[test]
fn rollup_ptu_only_optional_default() {
    let row = RollupMetricsRow {
        date: None,
        api_key: None,
        spend: None,
        ptu_flat_cost: None,
        prompt_tokens: None,
        completion_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        compression_saved_tokens: None,
        compression_savings_spend: None,
        prompt_caching_savings_spend: None,
        gateway_cache_savings_spend: None,
        autorouter_savings_spend: None,
        api_requests: None,
        successful_requests: None,
        failed_requests: None,
        total_response_time_ms: None,
        timed_requests: None,
    };
    assert_eq!(row.ptu_flat_cost, None);
}
