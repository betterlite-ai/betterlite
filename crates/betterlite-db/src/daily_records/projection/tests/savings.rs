#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::daily_records::DailyRecordError;
use crate::daily_records::support::{
    Case, column, payload_with, payload_with_metadata, price_record, project, projected_row,
};
use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::daily_spend_sql::values::SpendColumnValue;

fn cheaper_case() -> Case {
    Case::with_prices(BTreeMap::from([(
        "openai/gpt-4o".to_owned(),
        price_record(r#"{"input_cost_per_token": 0.5}"#),
    )]))
}

fn caching_case() -> Case {
    Case::with_prices(BTreeMap::from([(
        "openai/gpt-4o".to_owned(),
        price_record(r#"{"input_cost_per_token": 0.001, "cache_read_input_token_cost": 0.0001}"#),
    )]))
}

fn money(text: &str) -> SpendColumnValue {
    SpendColumnValue::Decimal(Decimal::from_str(text).unwrap())
}

#[tokio::test]
async fn prices_compression_tokens_at_the_injected_input_rate() {
    let text = payload_with_metadata(r#"{"compression_savings":{"tokens_saved":4}}"#);
    let row = projected_row(&cheaper_case(), &text, DailySpendEntity::User).await;
    assert_eq!(
        column(&row, "compression_saved_tokens"),
        SpendColumnValue::Int(4)
    );
    assert_eq!(column(&row, "compression_savings_spend"), money("2"));
    assert_eq!(
        serde_json::to_string(&column(&row, "compression_savings_spend")).unwrap(),
        "2.0"
    );
}

#[tokio::test]
async fn an_unpriced_model_earns_no_compression_savings() {
    let text = payload_with_metadata(r#"{"compression_savings":{"tokens_saved":4}}"#);
    let case = Case::with_prices(BTreeMap::new());
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_eq!(
        column(&row, "compression_saved_tokens"),
        SpendColumnValue::Int(4)
    );
    assert_eq!(column(&row, "compression_savings_spend"), money("0"));
}

#[tokio::test]
async fn keeps_the_recorded_autorouter_savings() {
    let text = payload_with_metadata(r#"{"autorouter_savings":0.5}"#);
    let row = projected_row(
        &Case::with_prices(BTreeMap::new()),
        &text,
        DailySpendEntity::User,
    )
    .await;
    assert_eq!(column(&row, "autorouter_savings_spend"), money("0.5"));
}

#[tokio::test]
async fn an_internal_call_zeroes_the_autorouter_savings_and_keeps_compression() {
    let text = payload_with_metadata(
        r#"{"internal_call_origin":"shadow_eval","autorouter_savings":0.5,"compression_savings":{"tokens_saved":4}}"#,
    );
    let row = projected_row(&cheaper_case(), &text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "autorouter_savings_spend"), money("0"));
    assert_eq!(column(&row, "compression_savings_spend"), money("2"));
}

fn cached_payload(marker: &str, model_id: &str) -> String {
    let metadata = format!(
        r#"{{"litellm_gateway_injected_cache":"{marker}","usage_object":{{"cache_read_input_tokens":100,"prompt_tokens":100}}}}"#
    );
    payload_with(&format!(
        "\"model_id\":\"{model_id}\",\"metadata\":{}",
        serde_json::to_string(&metadata).unwrap()
    ))
}

#[tokio::test]
async fn credits_gateway_injected_caching_when_the_marker_names_the_billed_deployment() {
    let case = caching_case();
    let text = cached_payload("dep-1", "dep-1");
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "prompt_caching_savings_spend"), money("0.09"));
    assert_eq!(
        column(&row, "gateway_injected_caching_savings_spend"),
        money("0.09")
    );
}

#[tokio::test]
async fn a_marker_for_another_deployment_credits_no_gateway_caching() {
    let case = caching_case();
    let text = cached_payload("dep-1", "dep-2");
    let row = projected_row(&case, &text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "prompt_caching_savings_spend"), money("0.09"));
    assert_eq!(
        column(&row, "gateway_injected_caching_savings_spend"),
        money("0")
    );
}

#[tokio::test]
async fn an_unusable_usage_object_only_zeroes_the_usage_driven_savings() {
    let text =
        payload_with_metadata(r#"{"autorouter_savings":0.5,"usage_object":{"prompt_tokens":[1]}}"#);
    let row = projected_row(&caching_case(), &text, DailySpendEntity::User).await;
    assert_eq!(column(&row, "prompt_caching_savings_spend"), money("0"));
    assert_eq!(column(&row, "autorouter_savings_spend"), money("0.5"));
    assert_eq!(
        column(&row, "cache_read_input_tokens"),
        SpendColumnValue::Int(0)
    );
}

#[rstest]
#[case::number("5")]
#[case::boolean("true")]
#[case::list("[]")]
#[case::object("{}")]
#[tokio::test]
async fn a_bill_time_that_is_not_text_fails_the_row(#[case] end_time: &str) {
    let text = payload_with(&format!("\"endTime\":{end_time}"));
    let result = project(
        &Case::with_prices(BTreeMap::new()),
        &text,
        DailySpendEntity::User,
    )
    .await;
    assert!(matches!(
        result,
        Err(DailyRecordError::InvalidField { field: "endTime" })
    ));
}
