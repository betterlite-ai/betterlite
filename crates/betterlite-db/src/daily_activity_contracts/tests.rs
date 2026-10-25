#![cfg(test)]
use super::identifiers::DailyActivityTable;
use super::{
    AggregatedRows, DailyActivityProxyReads, DailyActivityRow, DailyActivityScope,
    DailyActivityScopeInput, EntityRollupRow, ExportRow, ExportType, GroupingSetsRow,
    KeyMetadataRow, KeyPage, KeySpendRow, RollupMetricsRow, SpendLogsWindow, SqlParameter,
    SqlQuery,
};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime};
macro_rules! table_tests {
    ($($case:ident => ($table:ident, $field:literal, $relation:literal)),+ $(,)?) => {
        #[rstest]
        $(#[case::$case(DailyActivityTable::$table, $field, $relation)])+
        fn table_field_and_relation(#[case] table: DailyActivityTable, #[case] field: &str, #[case] relation: &str) {
            assert_eq!(table.metadata().entity_field, field);
            assert_eq!(table.metadata().relation, relation);
        }
    };
}
table_tests! {
    table_field_and_relation_user => (User, "user_id", "LiteLLM_DailyUserSpend"),
    table_field_and_relation_team => (Team, "team_id", "LiteLLM_DailyTeamSpend"),
    table_field_and_relation_tag => (Tag, "tag", "LiteLLM_DailyTagSpend"),
    table_field_and_relation_organization => (Organization, "organization_id", "LiteLLM_DailyOrganizationSpend"),
    table_field_and_relation_customer => (Customer, "end_user_id", "LiteLLM_DailyEndUserSpend"),
    table_field_and_relation_agent => (Agent, "agent_id", "LiteLLM_DailyAgentSpend"),
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
    assert!(!scope.as_ref().include_current_utc_day);
}
#[test]
fn scope_none_empty_and_duplicate_filters() {
    let scope = serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":["","a","a"],"exclude_entity_ids":["x","x"],"api_keys":[],"start_date":"a","end_date":"b","model":"","timezone_offset_minutes":0}"#).expect("scope");
    assert_eq!(scope.as_ref().api_keys, Some(Vec::new()));
}
#[test]
fn scope_dates_and_offsets_not_normalized() {
    let scope = serde_json::from_str::<DailyActivityScope>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"later","end_date":"earlier","model":null,"timezone_offset_minutes":-90}"#).expect("scope");
    assert_eq!(scope.as_ref().timezone_offset_minutes, Some(-90_i64));
}
#[test]
fn sql_parameter_order() {
    let query =
        serde_json::from_str::<SqlQuery>(r#"{"sql":"SELECT $1, $2","params":["a",2]}"#).unwrap();
    let params = vec![SqlParameter::Text("a".to_owned()), SqlParameter::Integer(2)];
    assert_eq!(query.params, params);
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
#[rstest]
#[case::nonfinite_money_rejected("NaN")]
#[case::money_outside_decimal_range_rejected("79228162514264337593543950336")]
fn invalid_money(#[case] value: &str) {
    serde_json::from_str::<Decimal>(&format!("\"{value}\"")).unwrap_err();
}
#[test]
fn decimal_money_exact() {
    let amount = serde_json::from_str::<Decimal>("\"0.1234567890123456789012345678\"").unwrap();
    assert_eq!(amount.to_string(), "0.1234567890123456789012345678");
}
#[test]
fn native_scope_construction() {
    let input = serde_json::from_str::<DailyActivityScopeInput>(r#"{"table":"litellm_dailyuserspend","entity_id_field":"user_id","entity_ids":null,"exclude_entity_ids":[],"api_keys":null,"start_date":"a","end_date":"b","model":null,"timezone_offset_minutes":0}"#).unwrap();
    let scope = DailyActivityScope::try_from(input).unwrap();
    assert_eq!(scope.as_ref().table, DailyActivityTable::User);
}
macro_rules! field_tests {
    ($prefix:ident, $row:ty, $fixture:literal; $(($nullable:literal, $default:literal) => [$($case:ident),+]);+ $(;)?) => {
        #[rstest]
        $($(#[case::$case(stringify!($case), ($nullable, $default))])+)+
        fn $prefix(#[case] name: &str, #[case] expected: (bool, bool)) {
            let field = name.strip_prefix(concat!(stringify!($prefix), "_field_")).unwrap();
            let original = include_str!($fixture);
            serde_json::from_str::<$row>(original).unwrap();
            let mut fields: BTreeMap<String, Box<serde_json::value::RawValue>> = serde_json::from_str(original).unwrap();
            fields.remove(field).unwrap();
            assert_eq!(serde_json::from_str::<$row>(&serde_json::to_string(&fields).unwrap()).is_ok(), expected.1);
            fields.insert(field.to_owned(), serde_json::value::RawValue::from_string("null".to_owned()).unwrap());
            assert_eq!(serde_json::from_str::<$row>(&serde_json::to_string(&fields).unwrap()).is_ok(), expected.0);
        }
    };
}
field_tests! { key_spend, KeySpendRow, "fixtures/key_spend.json";
    (false, false) => [key_spend_field_api_key, key_spend_field_spend, key_spend_field_prompt_tokens, key_spend_field_completion_tokens, key_spend_field_total_tokens, key_spend_field_api_requests, key_spend_field_successful_requests, key_spend_field_failed_requests, key_spend_field_cache_read_input_tokens, key_spend_field_cache_creation_input_tokens];
}
field_tests! { key_metadata, KeyMetadataRow, "fixtures/key_metadata.json";
    (false, false) => [key_metadata_field_api_key, key_metadata_field_key_exists, key_metadata_field_tags];
    (true, false) => [key_metadata_field_key_alias, key_metadata_field_team_id, key_metadata_field_user_id, key_metadata_field_user_email];
}
field_tests! { export, ExportRow, "fixtures/export.json";
    (false, false) => [export_field_date, export_field_entity_id, export_field_spend, export_field_flat_cost, export_field_prompt_tokens, export_field_completion_tokens, export_field_api_requests, export_field_successful_requests, export_field_failed_requests, export_field_cache_read_input_tokens, export_field_cache_creation_input_tokens];
    (true, false) => [export_field_entity_alias, export_field_api_key, export_field_key_alias, export_field_user_id, export_field_user_email, export_field_model];
}
field_tests! { rollup, RollupMetricsRow, "fixtures/rollup.json";
    (true, false) => [rollup_field_date, rollup_field_api_key, rollup_field_spend, rollup_field_prompt_tokens, rollup_field_completion_tokens, rollup_field_cache_read_input_tokens, rollup_field_cache_creation_input_tokens, rollup_field_compression_saved_tokens, rollup_field_compression_savings_spend, rollup_field_prompt_caching_savings_spend, rollup_field_gateway_injected_caching_savings_spend, rollup_field_autorouter_savings_spend, rollup_field_api_requests, rollup_field_successful_requests, rollup_field_failed_requests, rollup_field_total_response_time_ms, rollup_field_timed_requests];
    (true, true) => [rollup_field_ptu_flat_cost];
}
field_tests! { grouping, GroupingSetsRow, "fixtures/grouping.json";
    (true, false) => [grouping_field_date, grouping_field_api_key, grouping_field_spend, grouping_field_prompt_tokens, grouping_field_completion_tokens, grouping_field_cache_read_input_tokens, grouping_field_cache_creation_input_tokens, grouping_field_compression_saved_tokens, grouping_field_compression_savings_spend, grouping_field_prompt_caching_savings_spend, grouping_field_gateway_injected_caching_savings_spend, grouping_field_autorouter_savings_spend, grouping_field_api_requests, grouping_field_successful_requests, grouping_field_failed_requests, grouping_field_total_response_time_ms, grouping_field_timed_requests, grouping_field_model, grouping_field_model_group, grouping_field_custom_llm_provider, grouping_field_mcp_namespaced_tool_name, grouping_field_endpoint, grouping_field_distinct_api_keys];
    (true, true) => [grouping_field_ptu_flat_cost];
    (false, false) => [grouping_field_group_level];
}
field_tests! { entity, EntityRollupRow, "fixtures/entity.json";
    (true, false) => [entity_field_date, entity_field_api_key, entity_field_spend, entity_field_prompt_tokens, entity_field_completion_tokens, entity_field_cache_read_input_tokens, entity_field_cache_creation_input_tokens, entity_field_compression_saved_tokens, entity_field_compression_savings_spend, entity_field_prompt_caching_savings_spend, entity_field_gateway_injected_caching_savings_spend, entity_field_autorouter_savings_spend, entity_field_api_requests, entity_field_successful_requests, entity_field_failed_requests, entity_field_total_response_time_ms, entity_field_timed_requests, entity_field_entity_id, entity_field_distinct_api_keys];
    (true, true) => [entity_field_ptu_flat_cost];
    (false, false) => [entity_field_api_key_rolled];
}
field_tests! { daily_row, DailyActivityRow, "fixtures/daily_row.json";
    (false, false) => [daily_row_field_id, daily_row_field_date, daily_row_field_api_key, daily_row_field_prompt_tokens, daily_row_field_completion_tokens, daily_row_field_cache_read_input_tokens, daily_row_field_cache_creation_input_tokens, daily_row_field_compression_saved_tokens, daily_row_field_compression_savings_spend, daily_row_field_prompt_caching_savings_spend, daily_row_field_gateway_injected_caching_savings_spend, daily_row_field_autorouter_savings_spend, daily_row_field_spend, daily_row_field_api_requests, daily_row_field_successful_requests, daily_row_field_failed_requests, daily_row_field_total_response_time_ms, daily_row_field_timed_requests];
    (true, false) => [daily_row_field_model, daily_row_field_model_group, daily_row_field_custom_llm_provider, daily_row_field_mcp_namespaced_tool_name, daily_row_field_endpoint];
}
#[rstest]
#[case::daily(ExportType::Daily, "daily")]
#[case::keys(ExportType::DailyWithKeys, "daily_with_keys")]
#[case::models(ExportType::DailyWithModels, "daily_with_models")]
#[case::users(ExportType::DailyWithUsers, "daily_with_users")]
fn export_type_wire_spellings(#[case] kind: ExportType, #[case] spelling: &str) {
    assert_eq!(
        serde_json::to_string(&kind).unwrap(),
        format!("\"{spelling}\"")
    );
}
#[test]
fn entity_rows_absent_vs_empty() {
    let absent = serde_json::from_str::<AggregatedRows>(
        r#"{"grouping_rows":[],"entity_rows":null,"distinct_api_keys":0}"#,
    )
    .unwrap();
    let empty = serde_json::from_str::<AggregatedRows>(
        r#"{"grouping_rows":[],"entity_rows":[],"distinct_api_keys":0}"#,
    )
    .unwrap();
    assert_eq!(absent.entity_rows, None);
    assert_eq!(empty.entity_rows, Some(Vec::new()));
}
#[test]
fn page_order_preserved() {
    let page = serde_json::from_str::<KeyPage>(include_str!("fixtures/key_page.json")).unwrap();
    let keys: Vec<_> = page.rows.iter().map(|row| row.api_key.as_str()).collect();
    assert_eq!(keys, ["b", "a"]);
    assert_eq!(page.total_api_keys, 2);
}
struct Recovery(SpendLogsWindow);
impl DailyActivityProxyReads for Recovery {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> {
        assert_eq!(api_keys, &BTreeSet::from(["requested".to_owned()]));
        assert_eq!(window, Some(self.0));
        ready(resolved)
    }
}
#[test]
fn recovery_window_passed_unchanged() {
    let metadata =
        serde_json::from_str::<KeyMetadataRow>(include_str!("fixtures/key_metadata.json")).unwrap();
    let resolved = BTreeMap::from([("existing".to_owned(), metadata)]);
    let keys = BTreeSet::from(["requested".to_owned()]);
    let end = SystemTime::UNIX_EPOCH;
    let start = end.checked_add(Duration::from_secs(1)).unwrap();
    let reader = Recovery((start, end));
    let mut future = Box::pin(reader.recover_key_metadata(resolved.clone(), &keys, Some(reader.0)));
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(resolved));
}
