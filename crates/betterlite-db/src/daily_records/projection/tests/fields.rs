#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::daily_records::DailyRecordError;
use crate::daily_records::projection::common_daily_transaction;
use crate::daily_records::projection::rows::base_spend_row;
use crate::daily_records::support::{
    Case, base_payload, column, daily_payload, payload_with, project, projected_base, projected_row,
};
use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::daily_spend_sql::values::SpendColumnValue;

fn case() -> Case {
    Case::with_prices(BTreeMap::new())
}

#[rstest]
#[case::missing_start_time(
    r#"{"user":"u1","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o"}"#
)]
#[case::missing_api_key(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o"}"#
)]
#[case::missing_model_and_tool(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","custom_llm_provider":"openai","model_group":"gpt-4o"}"#
)]
#[case::model_without_a_provider(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","model_group":"gpt-4o"}"#
)]
#[case::model_without_a_group(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai"}"#
)]
#[tokio::test]
async fn skips_a_payload_that_is_missing_a_gate_key(#[case] text: &str) {
    let projection = project(&case(), text, DailySpendEntity::User)
        .await
        .unwrap();
    assert!(projection.is_none());
}

#[rstest]
#[case::tool_with_a_model_and_no_provider(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","mcp_namespaced_tool_name":"tool"}"#
)]
#[case::tool_alone(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","mcp_namespaced_tool_name":"tool"}"#
)]
#[case::tool_holding_null(
    r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","mcp_namespaced_tool_name":null}"#
)]
#[tokio::test]
async fn a_present_tool_key_skips_the_provider_and_group_requirement(#[case] text: &str) {
    let projection = project(&case(), text, DailySpendEntity::User)
        .await
        .unwrap();
    assert!(projection.is_some());
}

#[tokio::test]
async fn a_present_null_api_key_fails_the_row() {
    let text = r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":null,"model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o"}"#;
    let result = project(&case(), text, DailySpendEntity::User).await;
    assert!(matches!(
        result,
        Err(DailyRecordError::InvalidField { field: "api_key" })
    ));
}

#[tokio::test]
async fn a_present_null_start_time_builds_no_record() {
    let text = r#"{"user":"u1","startTime":null,"api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o"}"#;
    let projection = project(&case(), text, DailySpendEntity::User)
        .await
        .unwrap();
    assert!(projection.is_none());
}

#[tokio::test]
async fn a_null_user_still_rolls_up_under_a_missing_identity() {
    let mut payload = daily_payload(&base_payload());
    payload.user = Some(None);
    let case = case();
    let projection = common_daily_transaction(
        &payload,
        DailySpendEntity::User,
        &case.pricing(),
        &case.endpoints,
    )
    .await
    .unwrap()
    .expect("a present null user rolls up");
    assert_eq!(projection.identities, vec![None]);
}

#[tokio::test]
async fn the_end_user_base_reads_the_projected_end_user_id() {
    let payload_text = payload_with(r#""end_user_id":"e2""#);
    let payload = daily_payload(&payload_text);
    let case = case();
    let projection = common_daily_transaction(
        &payload,
        DailySpendEntity::EndUser,
        &case.pricing(),
        &case.endpoints,
    )
    .await
    .unwrap()
    .expect("the projected end user id names a base");
    assert_eq!(projection.identities, vec![Some("e2".to_owned())]);
}

#[tokio::test]
async fn an_end_user_without_the_projected_id_names_no_base() {
    let payload_text = payload_with(r#""end_user":"e1""#);
    let payload = daily_payload(&payload_text);
    let case = case();
    let projection = common_daily_transaction(
        &payload,
        DailySpendEntity::EndUser,
        &case.pricing(),
        &case.endpoints,
    )
    .await
    .unwrap();
    assert!(projection.is_none());
}

#[rstest]
#[case::timestamp("2025-01-02T03:04:05Z", "2025-01-02")]
#[case::date_only("2025-01-02", "2025-01-02")]
#[case::offset("2025-01-02T23:59:59-08:00", "2025-01-02")]
#[case::several_t("2025-01-02TaTb", "2025-01-02")]
#[case::empty("", "")]
#[tokio::test]
async fn takes_the_date_before_the_first_t(#[case] start_time: &str, #[case] date: &str) {
    let text = payload_with(&format!(r#""startTime":"{start_time}""#));
    let row = projected_row(&case(), &text, DailySpendEntity::User).await;
    assert_eq!(
        column(&row, "date"),
        SpendColumnValue::Text(date.to_owned())
    );
}

#[rstest]
#[case::number(r#""startTime":1700000000"#)]
#[case::boolean(r#""startTime":true"#)]
#[case::list(r#""startTime":[]"#)]
#[case::object(r#""startTime":{}"#)]
#[tokio::test]
async fn a_non_text_start_time_builds_no_record(#[case] fragment: &str) {
    let text = payload_with(fragment);
    let projection = project(&case(), &text, DailySpendEntity::User)
        .await
        .unwrap();
    assert!(projection.is_none());
}

#[tokio::test]
async fn maps_a_call_type_through_the_injected_route_map() {
    let mut case = case();
    case.endpoints.routes.insert(
        "aimage_generation".to_owned(),
        "/image/generations".to_owned(),
    );
    let text = payload_with(r#""call_type":"aimage_generation""#);
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_eq!(
        column(&row, "endpoint"),
        SpendColumnValue::Text("/image/generations".to_owned())
    );
}

#[rstest]
#[case::unmapped(r#""call_type":"acompletion""#)]
#[case::empty(r#""call_type":""#)]
#[case::missing("")]
#[tokio::test]
async fn an_unmapped_or_missing_call_type_has_no_endpoint(#[case] fragment: &str) {
    let mut case = case();
    case.endpoints
        .routes
        .insert(String::new(), "/empty".to_owned());
    let text = if fragment.is_empty() {
        base_payload()
    } else {
        payload_with(fragment)
    };
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "endpoint"), SpendColumnValue::Null);
}

#[tokio::test]
async fn carries_the_model_identity_and_money_of_the_logged_row() {
    let row = projected_row(&case(), &base_payload(), DailySpendEntity::User).await;
    assert_eq!(
        column(&row, "model"),
        SpendColumnValue::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        column(&row, "model_group"),
        SpendColumnValue::Text("gpt-4o".to_owned())
    );
    assert_eq!(
        column(&row, "custom_llm_provider"),
        SpendColumnValue::Text("openai".to_owned())
    );
    assert_eq!(column(&row, "prompt_tokens"), SpendColumnValue::Int(3));
    assert_eq!(column(&row, "completion_tokens"), SpendColumnValue::Int(4));
    assert_eq!(
        serde_json::to_string(&column(&row, "spend")).unwrap(),
        "0.25"
    );
}

#[tokio::test]
async fn a_missing_money_or_token_count_reads_as_zero() {
    let text = r#"{"user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","spend":null}"#;
    let row = projected_row(&case(), text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "prompt_tokens"), SpendColumnValue::Int(0));
    assert_eq!(
        serde_json::to_string(&column(&row, "spend")).unwrap(),
        "0.0"
    );
}

#[tokio::test]
async fn builds_independent_records_from_one_payload() {
    let case = case();
    let text = payload_with(r#""mcp_namespaced_tool_name":"tool""#);
    let first = projected_base(&case, &text, DailySpendEntity::User).await;
    let second = projected_base(&case, &text, DailySpendEntity::User).await;
    assert_eq!(base_spend_row(&first), base_spend_row(&second));
}
