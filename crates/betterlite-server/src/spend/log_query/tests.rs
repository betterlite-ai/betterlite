#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::{FromStr as _, from_utf8};

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::dates::{DateFormats, parse_date};
use super::filters::{EqualityColumn, EqualityFilter, MetadataFilter, MetadataOperator};
use super::prepare::SpendLogRoute;
use super::sql;
use super::validate::{CacheHitFilter, ChoiceError, SortColumn, SortDirection};
use super::{SpendLogQuery, decode_spend_log_query};

mod prepare;

const DATES: &str = "start_date=2024-01-02&end_date=2024-01-03";
const PAGE_MINIMUM: &str = "Input should be greater than or equal to 1";
const NOT_AN_INTEGER: &str =
    "Input should be a valid integer, unable to parse string as an integer";

fn decode(raw: &str) -> SpendLogQuery {
    decode_spend_log_query(Some(raw))
}

fn assert_single_issue(query: &SpendLogQuery, kind: &str, parameter: &str, msg: &str) {
    let mut issues = query.framework_issues().unwrap().issues;
    assert_eq!(issues.len(), 1);
    let issue = issues.remove(0);
    assert_eq!(issue.kind, kind);
    assert_eq!(issue.loc, ["query", parameter]);
    assert_eq!(issue.msg, msg);
}

fn utc(text: &str) -> DateTime<Utc> {
    parse_date(text, DateFormats::SecondsOnly).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

#[test]
fn decoding_defaults_page_page_size_and_content_filters() {
    let query = decode("");
    assert_eq!(query.page.page, 1);
    assert_eq!(query.page.page_size, 50);
    assert_eq!(query.page.session_cursor, None);
    assert_eq!(query.filters.api_key_filter, None);
    assert_eq!(query.filters.user_id, None);
    assert_eq!(query.filters.request_id, None);
    assert_eq!(query.filters.session_id, None);
    assert_eq!(query.filters.team_id, None);
    assert_eq!(query.filters.status_filter, None);
    assert_eq!(query.filters.model, None);
    assert_eq!(query.filters.model_id, None);
    assert_eq!(query.filters.model_group, None);
    assert_eq!(query.filters.key_alias, None);
    assert_eq!(query.filters.end_user, None);
    assert_eq!(query.filters.error_code, None);
    assert_eq!(query.filters.error_message, None);
    assert_eq!(query.filters.search, None);
    assert_eq!(query.dates.start_date, None);
    assert_eq!(query.dates.end_date, None);
    assert_eq!(query.spend.minimum_inclusive, None);
    assert_eq!(query.spend.maximum_inclusive, None);
    assert_eq!(query.choices.sort_by, "startTime");
    assert_eq!(query.choices.sort_order, None);
    assert_eq!(query.choices.cache_hit_filter, None);
    assert_eq!(query.choices.span_type, None);
    assert_eq!(query.flags.used_client_oauth_token, None);
    assert!(!query.flags.exclude_internal_health_checks);
    assert!(!query.flags.group_by_session);
    assert!(query.framework_issues().is_none());
}

#[test]
fn decoding_reads_every_scalar_filter() {
    let query = decode(
        "api_key=k&user_id=u&request_id=r&session_id=s&team_id=t&status_filter=failure&model=m&model_id=mi&model_group=mg&key_alias=ka&end_user=eu&error_code=500&error_message=boom&search=a%26b%3Dc&session_cursor=c",
    );
    assert_eq!(query.filters.api_key_filter.as_deref(), Some("k"));
    assert_eq!(query.filters.user_id.as_deref(), Some("u"));
    assert_eq!(query.filters.request_id.as_deref(), Some("r"));
    assert_eq!(query.filters.session_id.as_deref(), Some("s"));
    assert_eq!(query.filters.team_id.as_deref(), Some("t"));
    assert_eq!(query.filters.status_filter.as_deref(), Some("failure"));
    assert_eq!(query.filters.model.as_deref(), Some("m"));
    assert_eq!(query.filters.model_id.as_deref(), Some("mi"));
    assert_eq!(query.filters.model_group.as_deref(), Some("mg"));
    assert_eq!(query.filters.key_alias.as_deref(), Some("ka"));
    assert_eq!(query.filters.end_user.as_deref(), Some("eu"));
    assert_eq!(query.filters.error_code.as_deref(), Some("500"));
    assert_eq!(query.filters.error_message.as_deref(), Some("boom"));
    assert_eq!(query.filters.search.as_deref(), Some("a&b=c"));
    assert_eq!(query.page.session_cursor.as_deref(), Some("c"));
}

#[test]
fn decoding_keeps_the_last_value_of_a_repeated_scalar() {
    let query = decode("model=a&model=b&api_key=k1&api_key=k2");
    assert_eq!(query.filters.model.as_deref(), Some("b"));
    assert_eq!(query.filters.api_key_filter.as_deref(), Some("k2"));
}

#[rstest]
#[case::default("", 1, 50)]
#[case::explicit("page=3&page_size=100", 3, 100)]
#[case::lower_bounds("page=1&page_size=1", 1, 1)]
#[case::upper_page_size("page=2&page_size=1000", 2, 1000)]
#[case::leading_zero("page=02&page_size=020", 2, 20)]
#[case::zero_fraction("page=2&page_size=2.000", 2, 2)]
#[case::underscores("page=1_0&page_size=1_00", 10, 100)]
fn decoding_reads_pages(#[case] raw: &str, #[case] page: i64, #[case] page_size: i64) {
    let query = decode(raw);
    assert_eq!(query.page.page, page);
    assert_eq!(query.page.page_size, page_size);
    assert!(query.framework_issues().is_none());
}

#[rstest]
#[case::zero("page=0")]
#[case::negative("page=-1")]
#[case::negative_huge("page=-99999999999999999999")]
#[case::zero_fraction("page=000.000")]
fn page_below_one_reports_greater_than_equal(#[case] raw: &str) {
    let query = decode(raw);
    assert_eq!(query.page.page, 1);
    assert_single_issue(&query, "greater_than_equal", "page", PAGE_MINIMUM);
}

#[rstest]
#[case::text("page=abc")]
#[case::fraction("page=1.5")]
#[case::empty("page=")]
#[case::leading_point("page=.5")]
#[case::leading_underscore("page=_1")]
#[case::mixed_fraction("page=2.50")]
#[case::trailing_letter("page=1x")]
#[case::trailing_underscore("page=1_")]
fn page_rejects_text(#[case] raw: &str) {
    let query = decode(raw);
    assert_eq!(query.page.page, 1);
    assert_single_issue(&query, "int_parsing", "page", NOT_AN_INTEGER);
}

#[test]
fn page_above_the_signed_range_reports_int_parsing() {
    let query = decode("page=99999999999999999999");
    assert_eq!(query.page.page, 1);
    let issues = query.framework_issues().unwrap().issues;
    assert_eq!(issues[0].kind, "int_parsing");
    assert_eq!(issues[0].loc, ["query", "page"]);
}

#[rstest]
#[case::zero(
    "page_size=0",
    "greater_than_equal",
    "Input should be greater than or equal to 1"
)]
#[case::negative(
    "page_size=-2",
    "greater_than_equal",
    "Input should be greater than or equal to 1"
)]
#[case::negative_huge(
    "page_size=-99999999999999999999",
    "greater_than_equal",
    "Input should be greater than or equal to 1"
)]
#[case::above_limit(
    "page_size=1001",
    "less_than_equal",
    "Input should be less than or equal to 1000"
)]
#[case::huge(
    "page_size=99999999999999999999",
    "less_than_equal",
    "Input should be less than or equal to 1000"
)]
#[case::text(
    "page_size=abc",
    "int_parsing",
    "Input should be a valid integer, unable to parse string as an integer"
)]
#[case::fraction(
    "page_size=5.5",
    "int_parsing",
    "Input should be a valid integer, unable to parse string as an integer"
)]
#[case::empty(
    "page_size=",
    "int_parsing",
    "Input should be a valid integer, unable to parse string as an integer"
)]
fn page_size_issues_match_fastapi(#[case] raw: &str, #[case] kind: &str, #[case] msg: &str) {
    let query = decode(raw);
    assert_eq!(query.page.page_size, 50);
    assert_single_issue(&query, kind, "page_size", msg);
}

#[test]
fn numeric_issues_keep_the_declaration_order() {
    let query = decode(
        "min_spend=abc&max_spend=nan&page=0&page_size=2000&used_client_oauth_token=maybe&exclude_internal_health_checks=nope&group_by_session=x",
    );
    let issues = query.framework_issues().unwrap().issues;
    let locs: Vec<&str> = issues.iter().map(|entry| entry.loc[1]).collect();
    assert_eq!(
        locs,
        [
            "min_spend",
            "max_spend",
            "page",
            "page_size",
            "used_client_oauth_token",
            "exclude_internal_health_checks",
            "group_by_session",
        ]
    );
    let kinds: Vec<&str> = issues.iter().map(|entry| entry.kind).collect();
    assert_eq!(
        kinds,
        [
            "float_parsing",
            "finite_number",
            "greater_than_equal",
            "less_than_equal",
            "bool_parsing",
            "bool_parsing",
            "bool_parsing",
        ]
    );
}

#[rstest]
#[case::plain("1.25", "1.25")]
#[case::exponent("1e2", "100")]
#[case::signed_exponent("2E%2B1", "20")]
#[case::underscores("1_000.5", "1000.5")]
#[case::negative("-2.5", "-2.5")]
#[case::plus_sign("%2B3", "3")]
#[case::leading_point(".5", "0.5")]
#[case::trailing_point("5.", "5")]
#[case::zero_fraction("3.000", "3")]
#[case::zero("0", "0")]
#[case::underflow("1e-400", "0")]
#[case::zero_underflow("0e-400", "0")]
fn money_accepts_decimal_text(#[case] raw: &str, #[case] expected: &str) {
    let query = decode(&format!("min_spend={raw}"));
    assert!(query.framework_issues().is_none());
    assert_eq!(query.spend.minimum_inclusive, Some(decimal(expected)));
}

#[rstest]
#[case::text(
    "abc",
    "float_parsing",
    "Input should be a valid number, unable to parse string as a number"
)]
#[case::empty(
    "",
    "float_parsing",
    "Input should be a valid number, unable to parse string as a number"
)]
#[case::exponent_without_digits(
    "1e",
    "float_parsing",
    "Input should be a valid number, unable to parse string as a number"
)]
#[case::leading_underscore(
    "_1",
    "float_parsing",
    "Input should be a valid number, unable to parse string as a number"
)]
#[case::double_underscore(
    "1__0",
    "float_parsing",
    "Input should be a valid number, unable to parse string as a number"
)]
#[case::infinity("inf", "finite_number", "Input should be a finite number")]
#[case::nan("nan", "finite_number", "Input should be a finite number")]
#[case::overflow("1e400", "finite_number", "Input should be a finite number")]
#[case::beyond_decimal_range("1e29", "decimal_parsing", "Input should be a valid decimal")]
#[case::tiny_exponent("1e-29", "decimal_parsing", "Input should be a valid decimal")]
fn money_rejects_text_and_values_decimal_cannot_hold(
    #[case] raw: &str,
    #[case] kind: &str,
    #[case] msg: &str,
) {
    let query = decode(&format!("max_spend={raw}"));
    assert_eq!(query.spend.maximum_inclusive, None);
    assert_single_issue(&query, kind, "max_spend", msg);
}

#[rstest]
#[case::absent("", None)]
#[case::false_text("used_client_oauth_token=false", Some(false))]
#[case::true_text("used_client_oauth_token=TRUE", Some(true))]
#[case::zero("used_client_oauth_token=0", Some(false))]
#[case::one("used_client_oauth_token=1", Some(true))]
#[case::no("used_client_oauth_token=no", Some(false))]
#[case::yes("used_client_oauth_token=yes", Some(true))]
fn used_client_oauth_token_keeps_presence_separate_from_false(
    #[case] raw: &str,
    #[case] expected: Option<bool>,
) {
    assert_eq!(decode(raw).flags.used_client_oauth_token, expected);
}

#[rstest]
#[case::on("exclude_internal_health_checks=on")]
#[case::mixed_case("exclude_internal_health_checks=Yes")]
#[case::letter_t("exclude_internal_health_checks=t")]
fn exclude_internal_health_checks_accepts_pydantic_tokens(#[case] raw: &str) {
    assert!(decode(raw).flags.exclude_internal_health_checks);
}

#[test]
fn group_by_session_accepts_a_pydantic_token() {
    assert!(decode("group_by_session=Y").flags.group_by_session);
}

#[test]
fn boolean_flag_text_is_not_trimmed() {
    let query = decode("exclude_internal_health_checks=true%20");
    assert!(!query.flags.exclude_internal_health_checks);
    let issues = query.framework_issues().unwrap().issues;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "bool_parsing");
    assert_eq!(issues[0].loc, ["query", "exclude_internal_health_checks"]);
    assert_eq!(
        issues[0].msg,
        "Input should be a valid boolean, unable to interpret input"
    );
}

#[test]
fn boolean_flags_default_to_false_when_absent() {
    let flags = decode("").flags;
    assert!(!flags.exclude_internal_health_checks);
    assert!(!flags.group_by_session);
}

#[tokio::test]
async fn framework_issues_render_fastapi_422_detail() {
    let response = decode("page=0").framework_issues().unwrap().into_response();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"detail":[{"type":"greater_than_equal","loc":["query","page"],"msg":"Input should be greater than or equal to 1"}]}"#
    );
}

#[rstest]
#[case::spend("spend", SortColumn::Spend)]
#[case::total_tokens("total_tokens", SortColumn::TotalTokens)]
#[case::start_time("startTime", SortColumn::StartTime)]
#[case::end_time("endTime", SortColumn::EndTime)]
#[case::request_duration("request_duration_ms", SortColumn::RequestDurationMs)]
#[case::model("model", SortColumn::Model)]
#[case::ttft("ttft_ms", SortColumn::TtftMs)]
fn sort_by_accepts_every_documented_field(#[case] sort_by: &str, #[case] expected: SortColumn) {
    let raw = format!("sort_by={sort_by}");
    let choices = decode(&raw).choices().unwrap();
    assert_eq!(choices.sort_column, expected);
}

#[test]
fn sort_by_defaults_to_start_time() {
    assert_eq!(
        decode("").choices().unwrap().sort_column,
        SortColumn::StartTime
    );
}

#[rstest]
#[case::ascending("asc", SortDirection::Ascending)]
#[case::upper("DESC", SortDirection::Descending)]
#[case::mixed("Desc", SortDirection::Descending)]
fn sort_order_is_case_insensitive(#[case] raw: &str, #[case] expected: SortDirection) {
    let query = format!("sort_order={raw}");
    assert_eq!(decode(&query).choices().unwrap().sort_direction, expected);
}

#[test]
fn sort_order_defaults_to_descending() {
    assert_eq!(
        decode("").choices().unwrap().sort_direction,
        SortDirection::Descending
    );
}

#[rstest]
#[case::hit("hit", CacheHitFilter::Hit)]
#[case::miss("miss", CacheHitFilter::Miss)]
fn cache_hit_filter_accepts_only_hit_and_miss(#[case] raw: &str, #[case] expected: CacheHitFilter) {
    let query = format!("cache_hit_filter={raw}");
    assert_eq!(decode(&query).choices().unwrap().cache_hit, Some(expected));
}

#[test]
fn cache_hit_filter_is_case_sensitive() {
    let error = decode("cache_hit_filter=HIT").choices().unwrap_err();
    assert_eq!(error, ChoiceError::CacheHitFilter("HIT".to_owned()));
}

#[rstest]
#[case::llm(
    "llm",
    "(call_type NOT IN ('call_mcp_tool', 'list_mcp_tools') AND call_type != 'asend_message' AND call_type NOT IN ('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch'))"
)]
#[case::agent("agent", "call_type = 'asend_message'")]
#[case::mcp("mcp", "call_type IN ('call_mcp_tool', 'list_mcp_tools')")]
#[case::batch(
    "batch",
    "call_type IN ('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch')"
)]
fn span_type_conditions_pin_call_type_predicates(#[case] span_type: &str, #[case] expected: &str) {
    let raw = format!("span_type={span_type}");
    let choices = decode(&raw).choices().unwrap();
    assert_eq!(choices.span_condition, Some(expected));
}

#[test]
fn span_condition_lookup_accepts_exactly_the_four_classes() {
    assert_eq!(sql::span_type_sql_condition(None), None);
    assert_eq!(sql::span_type_sql_condition(Some("LLM")), None);
    assert_eq!(sql::span_type_sql_condition(Some("")), None);
}

#[test]
fn invalid_sort_by_names_the_parameter() {
    let error = decode("sort_by=Spend").choices().unwrap_err();
    assert_eq!(error, ChoiceError::SortBy("Spend".to_owned()));
    assert_eq!(
        error.to_string(),
        "Invalid sort_by: Spend. Must be one of: endTime, model, request_duration_ms, spend, startTime, total_tokens, ttft_ms"
    );
}

#[test]
fn invalid_sort_order_names_the_parameter() {
    let error = decode("sort_order=sideways").choices().unwrap_err();
    assert_eq!(error, ChoiceError::SortOrder("sideways".to_owned()));
    assert_eq!(
        error.to_string(),
        "Invalid sort_order: sideways. Must be one of: asc, desc"
    );
}

#[test]
fn invalid_cache_hit_filter_names_the_parameter() {
    let error = decode("cache_hit_filter=both").choices().unwrap_err();
    assert_eq!(error, ChoiceError::CacheHitFilter("both".to_owned()));
    assert_eq!(
        error.to_string(),
        "Invalid cache_hit_filter: both. Must be one of: hit, miss"
    );
}

#[test]
fn invalid_span_type_names_the_parameter() {
    let error = decode("span_type=trace").choices().unwrap_err();
    assert_eq!(error, ChoiceError::SpanType("trace".to_owned()));
    assert_eq!(
        error.to_string(),
        "Invalid span_type: trace. Must be one of: llm, agent, mcp, batch"
    );
}

#[rstest]
#[case::sort_by(ChoiceError::SortBy("x".to_owned()), "sort_by")]
#[case::sort_order(ChoiceError::SortOrder("x".to_owned()), "sort_order")]
#[case::cache_hit(ChoiceError::CacheHitFilter("x".to_owned()), "cache_hit_filter")]
#[case::span(ChoiceError::SpanType("x".to_owned()), "span_type")]
fn choice_errors_name_their_parameter(#[case] error: ChoiceError, #[case] parameter: &str) {
    assert_eq!(error.parameter(), parameter);
}

#[tokio::test]
async fn choice_error_is_a_bad_request_envelope_naming_the_parameter() {
    let response = decode("cache_hit_filter=both")
        .choices()
        .unwrap_err()
        .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"error":{"message":"Invalid cache_hit_filter: both. Must be one of: hit, miss","type":"bad_request","param":"cache_hit_filter","code":"400"}}"#
    );
}

#[test]
fn choices_report_sort_by_before_sort_order() {
    let error = decode("sort_by=nope&sort_order=nope")
        .choices()
        .unwrap_err();
    assert_eq!(error, ChoiceError::SortBy("nope".to_owned()));
}

fn status_json(status: Option<&str>) -> Option<String> {
    sql::status_filter_condition(status).map(|condition| serde_json::to_string(&condition).unwrap())
}

#[test]
fn status_condition_for_success_includes_null() {
    assert_eq!(
        status_json(Some("success")).as_deref(),
        Some(r#"{"OR":[{"status":{"equals":"success"}},{"status":null}]}"#)
    );
}

#[test]
fn status_condition_for_other_statuses_is_exact() {
    assert_eq!(
        status_json(Some("failure")).as_deref(),
        Some(r#"{"status":{"equals":"failure"}}"#)
    );
}

#[test]
fn status_condition_for_null_is_absent() {
    assert_eq!(status_json(None), None);
}

#[test]
fn status_condition_for_empty_text_is_exact() {
    assert_eq!(
        status_json(Some("")).as_deref(),
        Some(r#"{"status":{"equals":""}}"#)
    );
}

#[test]
fn search_condition_pins_the_litellm_sql() {
    let condition = sql::spend_log_search_condition(
        "abc",
        utc("2024-01-02 03:04:05"),
        utc("2024-01-03 04:05:06"),
        3,
    );
    assert_eq!(
        condition.sql,
        "(request_id = $3 OR (\"startTime\" >= ($4::timestamptz AT TIME ZONE 'UTC') AND \"startTime\" <= ($5::timestamptz AT TIME ZONE 'UTC') AND (litellm_call_id = $3 OR api_key = $3 OR team_id = $3 OR \"user\" = $3 OR end_user = $3 OR session_id = $3 OR model_id = $3)))"
    );
    assert_eq!(condition.params.0, "abc");
    assert_eq!(condition.params.1, utc("2024-01-02 03:04:05"));
    assert_eq!(condition.params.2, utc("2024-01-03 04:05:06"));
}

#[test]
fn search_condition_uses_the_supplied_placeholder_index() {
    let start = utc("2024-01-02 03:04:05");
    let end = utc("2024-01-03 04:05:06");
    let first = sql::spend_log_search_condition("x", start, end, 1);
    assert!(first.sql.contains("(request_id = $1 OR ("));
    assert!(first.sql.contains("($2::timestamptz"));
    assert!(first.sql.contains("($3::timestamptz"));
    let second = sql::spend_log_search_condition("x", start, end, 10);
    assert!(second.sql.contains("(request_id = $10 OR ("));
    assert!(second.sql.contains("($11::timestamptz"));
    assert!(second.sql.contains("($12::timestamptz"));
}

#[test]
fn search_condition_keeps_the_search_text_in_the_parameters() {
    let condition = sql::spend_log_search_condition(
        "$1&x=y",
        utc("2024-01-02 03:04:05"),
        utc("2024-01-03 04:05:06"),
        2,
    );
    assert_eq!(condition.params.0, "$1&x=y");
    assert!(!condition.sql.contains("$1&x=y"));
    assert!(!condition.sql.contains("x=y"));
}

#[test]
fn session_expressions_pin_the_litellm_fragments() {
    assert_eq!(
        sql::SESSION_KEY_EXPR,
        "COALESCE(NULLIF(session_id, ''), request_id)"
    );
    assert_eq!(
        sql::SESSION_GROUP_KEY_SQL,
        "COALESCE(NULLIF(session_id, ''), request_id), api_key"
    );
    assert_eq!(
        sql::REPRESENTATIVE_ORDER_SQL,
        "(call_type = 'asend_message') DESC, CASE WHEN call_type = 'asend_message' THEN \"endTime\" END DESC NULLS LAST, call_type IN ('call_mcp_tool', 'list_mcp_tools'), \"startTime\" DESC, request_id"
    );
    assert_eq!(
        sql::MCP_CALL_TYPES_SQL,
        "('call_mcp_tool', 'list_mcp_tools')"
    );
    assert_eq!(sql::AGENT_CALL_TYPE_SQL, "'asend_message'");
    assert_eq!(
        sql::BATCH_CALL_TYPES_SQL,
        "('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch')"
    );
}

#[rstest]
#[case::identifiers("request_id, call_type, api_key, spend, total_tokens, ")]
#[case::tokens("prompt_tokens, completion_tokens, \"startTime\", \"endTime\", ")]
#[case::timestamps("\"completionStartTime\", model, model_id, model_group, ")]
#[case::provider("custom_llm_provider, api_base, \"user\", metadata, ")]
#[case::cache("cache_hit, cache_key, request_tags, team_id, ")]
#[case::owner("organization_id, end_user, requester_ip_address, ")]
#[case::session("session_id, status, mcp_namespaced_tool_name, agent_id, ")]
#[case::call("litellm_call_id, ")]
fn list_columns_carry_the_lean_fields(#[case] fragment: &str) {
    assert!(sql::SPEND_LOG_LIST_COLUMNS.contains(fragment));
}

#[rstest]
#[case::messages("messages")]
#[case::response("response")]
#[case::request_body("proxy_server_request")]
fn list_columns_leave_out_the_heavy_bodies(#[case] field: &str) {
    assert!(!sql::SPEND_LOG_LIST_COLUMNS.contains(field));
}

#[test]
fn list_columns_carry_the_duration_fallback() {
    assert!(sql::SPEND_LOG_LIST_COLUMNS.contains("COALESCE(request_duration_ms, "));
    assert!(sql::SPEND_LOG_LIST_COLUMNS.contains(
        "(EXTRACT(EPOCH FROM (\"endTime\" - \"startTime\")) * 1000)::INTEGER) AS request_duration_ms"
    ));
}

#[test]
fn pagination_cap_is_ten_thousand() {
    assert_eq!(sql::SPEND_LOGS_COUNT_CAP, 10_000);
}

#[test]
fn health_check_keys_are_the_name_and_its_sha256() {
    assert_eq!(
        sql::HEALTH_CHECK_SERVICE_ACCOUNT,
        "litellm-internal-health-check"
    );
    assert_eq!(
        sql::internal_health_check_api_keys(),
        [
            "litellm-internal-health-check".to_owned(),
            "2ee659761968def976c7905217d96fa6e765f300dfb066f6c73fd3c279b3435a".to_owned(),
        ]
    );
}

#[test]
fn prepare_builds_equality_filters_in_where_condition_order() {
    let query = format!(
        "end_user=eu&model_group=mg&model_id=mi&model=m&api_key=k&user_id=u&team_id=t&{DATES}"
    );
    let prepared = decode(&query).prepare(SpendLogRoute::PublicV2).unwrap();
    let columns: Vec<EqualityColumn> = prepared
        .equality
        .iter()
        .map(|filter| filter.column)
        .collect();
    assert_eq!(
        columns,
        [
            EqualityColumn::TeamId,
            EqualityColumn::ApiKey,
            EqualityColumn::User,
            EqualityColumn::Model,
            EqualityColumn::ModelId,
            EqualityColumn::ModelGroup,
            EqualityColumn::EndUser,
        ]
    );
    let values: Vec<&str> = prepared
        .equality
        .iter()
        .map(|filter| filter.value.as_str())
        .collect();
    assert_eq!(values, ["t", "k", "u", "m", "mi", "mg", "eu"]);
}

#[test]
fn prepare_maps_every_equality_column_to_its_sql_name() {
    assert_eq!(EqualityColumn::TeamId.sql_column(), "team_id");
    assert_eq!(EqualityColumn::User.sql_column(), "\"user\"");
    assert_eq!(EqualityColumn::ApiKey.sql_column(), "api_key");
    assert_eq!(EqualityColumn::Model.sql_column(), "model");
    assert_eq!(EqualityColumn::ModelId.sql_column(), "model_id");
    assert_eq!(EqualityColumn::ModelGroup.sql_column(), "model_group");
    assert_eq!(EqualityColumn::EndUser.sql_column(), "end_user");
}

#[test]
fn prepare_includes_present_empty_values_and_skips_absent_ones() {
    let query = format!("team_id=&model=m&{DATES}");
    let prepared = decode(&query).prepare(SpendLogRoute::PublicV2).unwrap();
    assert_eq!(
        prepared.equality,
        [
            EqualityFilter {
                column: EqualityColumn::TeamId,
                value: String::new(),
            },
            EqualityFilter {
                column: EqualityColumn::Model,
                value: "m".to_owned(),
            },
        ]
    );
}

#[test]
fn prepare_keeps_request_id_out_of_the_equality_filters() {
    let query = format!("request_id=r&{DATES}");
    let prepared = decode(&query).prepare(SpendLogRoute::PublicV2).unwrap();
    assert!(prepared.equality.is_empty());
    assert_eq!(prepared.request_id.as_deref(), Some("r"));
}

#[test]
fn prepare_orders_metadata_filters_like_litellm() {
    let query = format!("error_message=boom&error_code=500&key_alias=ka&{DATES}");
    let prepared = decode(&query).prepare(SpendLogRoute::PublicV2).unwrap();
    assert_eq!(
        prepared.metadata,
        [
            MetadataFilter::KeyAlias("ka".to_owned()),
            MetadataFilter::ErrorCode("500".to_owned()),
            MetadataFilter::ErrorMessage("boom".to_owned()),
        ]
    );
    assert_eq!(
        prepared.metadata[0].sql_column(),
        "metadata->>'user_api_key_alias'"
    );
    assert_eq!(prepared.metadata[0].operator(), MetadataOperator::Like);
    assert_eq!(prepared.metadata[0].parameter(), "%ka%");
    assert_eq!(
        prepared.metadata[1].sql_column(),
        "metadata->'error_information'->>'error_code'"
    );
    assert_eq!(prepared.metadata[1].operator(), MetadataOperator::Equals);
    assert_eq!(prepared.metadata[1].parameter(), "500");
    assert_eq!(
        prepared.metadata[2].sql_column(),
        "metadata->'error_information'->>'error_message'"
    );
    assert_eq!(prepared.metadata[2].operator(), MetadataOperator::Like);
    assert_eq!(prepared.metadata[2].parameter(), "%boom%");
    assert_eq!(MetadataOperator::Equals.sql(), "=");
    assert_eq!(MetadataOperator::Like.sql(), "LIKE");
}

#[test]
fn prepare_keeps_spend_bounds_inclusive() {
    let query = format!("min_spend=1.5&max_spend=2.5&{DATES}");
    let prepared = decode(&query).prepare(SpendLogRoute::PublicV2).unwrap();
    assert_eq!(prepared.spend.minimum_inclusive, Some(decimal("1.5")));
    assert_eq!(prepared.spend.maximum_inclusive, Some(decimal("2.5")));
}

#[rstest]
fn replays_the_spend_log_query_corpus(#[files("fuzz/corpus/spend_log_query/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let raw = from_utf8(&bytes).unwrap();
    let query = decode(raw);
    drop(query.choices());
    drop(query.prepare(SpendLogRoute::Ui));
    drop(query.prepare(SpendLogRoute::PublicV2));
    let page = query.page.page;
    let page_size = query.page.page_size;
    let again = decode(&format!("page={page}&page_size={page_size}"));
    assert_eq!(again.page.page, page);
    assert_eq!(again.page.page_size, page_size);
    assert!(again.framework_issues().is_none());
}
