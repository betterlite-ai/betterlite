#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::Config;
use betterlite_core::cost::prepare::BillingTokenCounter as _;
use betterlite_models::catalog::{BundledCatalog, CatalogError};
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;
use tower::ServiceExt as _;

use crate::cost_estimate_route::decode::decode_cost_estimate_request;
use crate::cost_estimate_route::deployments::DeploymentSet;
use crate::cost_estimate_route::error::EstimateError;
use crate::cost_estimate_route::estimate::NeverCounts;
use crate::cost_estimate_route::estimate::estimate;
use crate::cost_estimate_route::testing;
use crate::proxy_request_contracts::cost_estimate::{CostEstimateRequest, CostEstimateResponse};
use crate::routes;
use crate::routes::tests::state_for;
use crate::state::cost_service::{Slot, SlotError};

const AUTH: &str = "general_settings:\n  master_key: oracle-master\n";
const ALIAS: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: gpt-4o-mini-deployment\n    litellm_params: {model: openai/gpt-4o-mini}\nrouter_settings:\n  model_group_alias:\n    quote-alias: gpt-4o-mini-deployment\n";
const O3: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: o3-mini\n    litellm_params: {model: o3-mini}\n";
const VOLCENGINE: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: doubao-tier\n    litellm_params: {model: volcengine/doubao-seed-2-0-pro-260215}\n";
const GPT4O: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: gpt-4o\n    litellm_params: {model: openai/gpt-4o}\n";
const PROVIDER: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: mini-by-provider\n    litellm_params: {model: gpt-4o-mini, custom_llm_provider: openai}\n";
const OPENROUTER: &str = "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: openrouter-offpeak\n    litellm_params: {model: openrouter/deepseek/deepseek-v4-pro-0813}\n";

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
}

fn application(config: &Config) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    )
}

async fn post(application: Router, body: &str, key: Option<&str>) -> (StatusCode, String) {
    let mut request =
        Request::post("/cost/estimate").header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = key {
        request = request.header(header::AUTHORIZATION, format!("Bearer {value}"));
    }
    let response = application
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn quote(
    body: &'static str,
    source: &'static str,
) -> Pin<Box<dyn Future<Output = CostEstimateResponse>>> {
    Box::pin(async move {
        let (status, text) = post(application(&config(source)), body, Some("oracle-master")).await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_str(&text).unwrap()
    })
}

fn helper_quote(records: &str, request: &str) -> CostEstimateResponse {
    let rows: Vec<JsonObject> = serde_json::from_str(records).unwrap();
    let body: JsonValue = serde_json::from_str(request).unwrap();
    let value = testing::quote(&rows, &body).unwrap();
    serde_json::from_value(value).unwrap()
}

#[test]
fn decode_accepts_the_declared_fields() {
    let request = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,
            "cache_creation_input_tokens":1,"reasoning_tokens":1,"num_requests_per_day":0,
            "num_requests_per_month":4}"#,
    )
    .unwrap();
    assert_eq!(request.model, "m");
    assert_eq!(request.input_tokens, 8);
    assert_eq!(request.output_tokens, 4);
    assert_eq!(request.cache_read_input_tokens, 2);
    assert_eq!(request.cache_creation_input_tokens, 1);
    assert_eq!(request.reasoning_tokens, 1);
    assert_eq!(request.num_requests_per_day, Some(0));
    assert_eq!(request.num_requests_per_month, Some(4));
}

#[test]
fn decode_ignores_unknown_keys() {
    let request = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":1,"output_tokens":1,"extra":5}"#,
    )
    .unwrap();
    assert_eq!(request.model, "m");
}

#[tokio::test]
async fn answers_missing_required_fields_at_the_route() {
    let (status, body) = post(application(&config(AUTH)), "{}", Some("oracle-master")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"missing","loc":["body","model"],"msg":"Field required"},{"type":"missing","loc":["body","input_tokens"],"msg":"Field required"},{"type":"missing","loc":["body","output_tokens"],"msg":"Field required"}]}"#
    );
}

#[tokio::test]
async fn answers_an_empty_body_at_the_route() {
    let (status, body) = post(application(&config(AUTH)), "", Some("oracle-master")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required"}]}"#
    );
}

#[test]
fn decode_rejects_invalid_json() {
    let error = decode_cost_estimate_request(b"{").unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues) if issues[0].kind == "json_invalid"));
}

#[test]
fn decode_rejects_a_non_utf8_body() {
    let error = decode_cost_estimate_request(b"\xff\xfe").unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues) if issues[0].kind == "json_invalid"));
}

#[test]
fn decode_rejects_a_non_object_body() {
    let error = decode_cost_estimate_request(b"[]").unwrap_err();
    assert!(
        matches!(&error, EstimateError::Request(issues) if issues[0].kind == "model_attributes_type")
    );
}

#[test]
fn decode_rejects_a_wrong_typed_model() {
    let error =
        decode_cost_estimate_request(br#"{"model":null,"input_tokens":1,"output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "string_type" && issues[0].loc == ["body", "model"]));
}

#[test]
fn decode_rejects_a_null_count() {
    let error =
        decode_cost_estimate_request(br#"{"model":"m","input_tokens":null,"output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "int_type" && issues[0].msg == "Input should be a valid integer"));
}

#[test]
fn decode_rejects_a_string_count() {
    let error =
        decode_cost_estimate_request(br#"{"model":"m","input_tokens":"5","output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "int_type" && issues[0].loc == ["body", "input_tokens"]));
}

#[test]
fn decode_rejects_a_fractional_count() {
    let error =
        decode_cost_estimate_request(br#"{"model":"m","input_tokens":5.5,"output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues) if issues[0].kind == "int_type"));
}

#[test]
fn decode_rejects_a_boolean_count() {
    let error =
        decode_cost_estimate_request(br#"{"model":"m","input_tokens":true,"output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues) if issues[0].kind == "int_type"));
}

#[test]
fn decode_rejects_a_negative_count() {
    let error =
        decode_cost_estimate_request(br#"{"model":"m","input_tokens":-1,"output_tokens":1}"#)
            .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "greater_than_equal"
            && issues[0].msg == "Input should be greater than or equal to 0"));
}

#[test]
fn decode_rejects_a_negative_cache_count() {
    let error = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":10,"output_tokens":5,"cache_read_input_tokens":-1}"#,
    )
    .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "greater_than_equal" && issues[0].loc == ["body", "cache_read_input_tokens"]));
}

#[test]
fn decode_rejects_a_negative_period_count() {
    let error = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":1,"output_tokens":1,"num_requests_per_day":-1}"#,
    )
    .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues[0].kind == "greater_than_equal" && issues[0].loc == ["body", "num_requests_per_day"]));
}

#[test]
fn decode_treats_a_null_period_count_as_absent() {
    let request = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":1,"output_tokens":1,"num_requests_per_month":null}"#,
    )
    .unwrap();
    assert_eq!(request.num_requests_per_month, None);
}

#[test]
fn decode_reports_the_cache_subset_before_the_reasoning_subset() {
    let error = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":10,"output_tokens":5,"cache_read_input_tokens":8,
            "cache_creation_input_tokens":4,"reasoning_tokens":6}"#,
    )
    .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues.len() == 1
            && issues[0].kind == "value_error"
            && issues[0].msg == "Value error, cache_read_input_tokens plus cache_creation_input_tokens cannot exceed input_tokens"));
}

#[test]
fn decode_reports_the_reasoning_subset() {
    let error = decode_cost_estimate_request(
        br#"{"model":"m","input_tokens":10,"output_tokens":5,"reasoning_tokens":6}"#,
    )
    .unwrap_err();
    assert!(matches!(&error, EstimateError::Request(issues)
        if issues.len() == 1 && issues[0].msg == "Value error, reasoning_tokens cannot exceed output_tokens"));
}

#[tokio::test]
async fn answers_the_validation_issues_at_the_route() {
    let (status, body) = post(
        application(&config(AUTH)),
        r#"{"model":"x","input_tokens":10,"output_tokens":5,"cache_read_input_tokens":8,"cache_creation_input_tokens":4,"reasoning_tokens":6}"#,
        Some("oracle-master"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"value_error","loc":["body"],"msg":"Value error, cache_read_input_tokens plus cache_creation_input_tokens cannot exceed input_tokens"}]}"#
    );
}

#[tokio::test]
async fn answers_invalid_json_at_the_route() {
    let (status, body) = post(application(&config(AUTH)), "{", Some("oracle-master")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#
    );
}

#[tokio::test]
async fn requires_the_master_key() {
    let (status, _) = post(application(&config(AUTH)), "{}", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn rejects_a_wrong_key() {
    let (status, body) = post(application(&config(AUTH)), "{}", Some("wrong-key")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("no_db_connection"));
}

#[tokio::test]
async fn answers_an_authenticated_quote_for_a_catalog_model() {
    let response = quote(
        r#"{"model":"gpt-4o","input_tokens":1000,"output_tokens":500}"#,
        GPT4O,
    )
    .await;
    assert_eq!(response.cost_per_request.0, dec("0.0075"));
    assert_eq!(response.input_cost_per_request.0, dec("0.0025"));
    assert_eq!(response.output_cost_per_request.0, dec("0.005"));
    assert_eq!(response.margin_cost_per_request.0, Decimal::ZERO);
    assert_eq!(response.input_cost_per_token.unwrap().0, dec("0.0000025"));
    assert_eq!(response.output_cost_per_token.unwrap().0, dec("0.00001"));
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.00000125")
    );
    assert_eq!(
        response.cache_creation_token_cost.unwrap().0,
        dec("0.0000025")
    );
    assert_eq!(
        response.output_reasoning_token_cost.unwrap().0,
        dec("0.00001")
    );
    assert_eq!(response.provider.as_deref(), Some("openai"));
    assert_eq!(response.daily_cost, None);
    assert_eq!(response.monthly_cost, None);
}

#[tokio::test]
async fn quotes_a_model_group_alias_through_its_deployment() {
    let response = quote(
        r#"{"model":"quote-alias","input_tokens":1024,"output_tokens":256}"#,
        ALIAS,
    )
    .await;
    assert_eq!(response.model, "quote-alias");
    assert_eq!(response.cost_per_request.0, dec("0.0003072"));
    assert_eq!(response.input_cost_per_request.0, dec("0.0001536"));
    assert_eq!(response.output_cost_per_request.0, dec("0.0001536"));
    assert_eq!(response.input_cost_per_token.unwrap().0, dec("0.00000015"));
    assert_eq!(response.output_cost_per_token.unwrap().0, dec("0.0000006"));
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.000000075")
    );
    assert_eq!(
        response.cache_creation_token_cost.unwrap().0,
        dec("0.00000015")
    );
    assert_eq!(response.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn bills_cache_and_reasoning_tokens_as_subsets() {
    let response = quote(
        r#"{"model":"o3-mini","input_tokens":4096,"output_tokens":1024,"cache_read_input_tokens":1024,
            "cache_creation_input_tokens":1024,"reasoning_tokens":512,"num_requests_per_day":0,
            "num_requests_per_month":4}"#,
        O3,
    )
    .await;
    assert_eq!(response.cost_per_request.0, dec("0.008448"));
    assert_eq!(response.input_cost_per_request.0, dec("0.0039424"));
    assert_eq!(response.output_cost_per_request.0, dec("0.0045056"));
    assert_eq!(response.cache_read_cost_per_request.0, dec("0.0005632"));
    assert_eq!(response.cache_creation_cost.0, dec("0.0011264"));
    assert_eq!(response.reasoning_cost_per_request.0, dec("0.0022528"));
    assert_eq!(response.input_cost_per_token.unwrap().0, dec("0.0000011"));
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.00000055")
    );
    assert_eq!(
        response.cache_creation_token_cost.unwrap().0,
        dec("0.0000011")
    );
    assert_eq!(
        response.output_reasoning_token_cost.unwrap().0,
        dec("0.0000044")
    );
    assert_eq!(response.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn scales_the_monthly_lines_but_answers_a_zero_daily_count_with_nulls() {
    let response = quote(
        r#"{"model":"o3-mini","input_tokens":4096,"output_tokens":1024,"cache_read_input_tokens":1024,
            "cache_creation_input_tokens":1024,"reasoning_tokens":512,"num_requests_per_day":0,
            "num_requests_per_month":4}"#,
        O3,
    )
    .await;
    assert_eq!(response.daily_cost, None);
    assert_eq!(response.daily_input_cost, None);
    assert_eq!(response.monthly_cost.unwrap().0, dec("0.033792"));
    assert_eq!(response.monthly_input_cost.unwrap().0, dec("0.0157696"));
    assert_eq!(response.monthly_output_cost.unwrap().0, dec("0.0180224"));
    assert_eq!(response.monthly_margin_cost.unwrap().0, Decimal::ZERO);
    assert_eq!(
        response.monthly_cache_read_cost.unwrap().0,
        dec("0.0022528")
    );
    assert_eq!(
        response.monthly_cache_creation_cost.unwrap().0,
        dec("0.0045056")
    );
    assert_eq!(response.monthly_reasoning_cost.unwrap().0, dec("0.0090112"));
}

#[tokio::test]
async fn reports_the_tier_rates_the_quote_was_billed_at() {
    let response = quote(
        r#"{"model":"doubao-tier","input_tokens":64000,"output_tokens":1000}"#,
        VOLCENGINE,
    )
    .await;
    assert_eq!(response.cost_per_request.0, dec("0.0483"));
    assert_eq!(response.input_cost_per_request.0, dec("0.0448"));
    assert_eq!(response.output_cost_per_request.0, dec("0.0035"));
    assert_eq!(response.input_cost_per_token.unwrap().0, dec("0.0000007"));
    assert_eq!(response.output_cost_per_token.unwrap().0, dec("0.0000035"));
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.0000007")
    );
    assert_eq!(
        response.cache_creation_token_cost.unwrap().0,
        dec("0.0000007")
    );
    assert_eq!(
        response.output_reasoning_token_cost.unwrap().0,
        dec("0.0000035")
    );
    assert_eq!(response.provider.as_deref(), Some("volcengine"));
}

#[tokio::test]
async fn quotes_a_deployment_provider_from_the_config() {
    let response = quote(
        r#"{"model":"mini-by-provider","input_tokens":1024,"output_tokens":256}"#,
        PROVIDER,
    )
    .await;
    assert_eq!(response.cost_per_request.0, dec("0.0003072"));
    assert_eq!(response.provider.as_deref(), Some("openai"));
}

#[test]
fn quotes_the_deployment_prices_before_the_model_info_prices() {
    let response = helper_quote(
        r#"[{"model_name":"on-prem-quote","litellm_params":{"model":"claude-haiku-4-5-20251001",
            "custom_llm_provider":"anthropic","input_cost_per_token":0.5,"output_cost_per_token":1.5,
            "cache_read_input_token_cost":0.25},"model_info":{"input_cost_per_token":9.0,
            "output_cost_per_token":9.0,"cache_creation_input_token_cost":0.25}}]"#,
        r#"{"model":"on-prem-quote","input_tokens":8,"output_tokens":4,"cache_read_input_tokens":4,
            "cache_creation_input_tokens":2,"reasoning_tokens":1}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("8.5"));
    assert_eq!(response.input_cost_per_request.0, dec("2.5"));
    assert_eq!(response.output_cost_per_request.0, dec("6"));
    assert_eq!(response.cache_read_cost_per_request.0, dec("1"));
    assert_eq!(response.cache_creation_cost.0, dec("0.5"));
    assert_eq!(response.reasoning_cost_per_request.0, dec("1.5"));
    assert_eq!(response.input_cost_per_token.unwrap().0, dec("0.5"));
    assert_eq!(response.output_cost_per_token.unwrap().0, dec("1.5"));
    assert_eq!(response.cache_read_input_token_cost.unwrap().0, dec("0.25"));
    assert_eq!(response.cache_creation_token_cost.unwrap().0, dec("0.25"));
    assert_eq!(response.output_reasoning_token_cost.unwrap().0, dec("1.5"));
    assert_eq!(response.provider.as_deref(), Some("anthropic"));
}

#[test]
fn keeps_an_explicit_zero_cache_write_rate() {
    let response = helper_quote(
        r#"[{"model_name":"zero-cache","litellm_params":{"model":"claude-haiku-4-5-20251001",
            "custom_llm_provider":"anthropic","input_cost_per_token":0.5,"output_cost_per_token":1.5,
            "cache_creation_input_token_cost":0}}]"#,
        r#"{"model":"zero-cache","input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,
            "cache_creation_input_tokens":2,"reasoning_tokens":1}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("8.0000002"));
    assert_eq!(response.input_cost_per_request.0, dec("2.0000002"));
    assert_eq!(response.cache_read_cost_per_request.0, dec("0.0000002"));
    assert_eq!(response.cache_creation_cost.0, Decimal::ZERO);
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.0000001")
    );
    assert_eq!(response.cache_creation_token_cost.unwrap().0, Decimal::ZERO);
}

#[test]
fn quotes_a_partial_custom_price_with_a_zero_for_the_missing_rate() {
    let response = helper_quote(
        r#"[{"model_name":"input-only","litellm_params":{"model":"self-hosted",
            "custom_llm_provider":"openai","input_cost_per_token":0.5}}]"#,
        r#"{"model":"input-only","input_tokens":8,"output_tokens":4}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("4"));
    assert_eq!(response.output_cost_per_request.0, Decimal::ZERO);
    assert_eq!(response.output_cost_per_token.unwrap().0, Decimal::ZERO);
}

#[test]
fn falls_back_to_the_custom_input_rate_for_cache_tokens() {
    let response = helper_quote(
        r#"[{"model_name":"input-only","litellm_params":{"model":"self-hosted",
            "custom_llm_provider":"openai","input_cost_per_token":0.5}}]"#,
        r#"{"model":"input-only","input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,
            "cache_creation_input_tokens":2}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("4"));
    assert_eq!(response.input_cost_per_request.0, dec("4"));
    assert_eq!(response.output_cost_per_request.0, Decimal::ZERO);
    assert_eq!(response.cache_read_cost_per_request.0, dec("1"));
    assert_eq!(response.cache_creation_cost.0, dec("1"));
    assert_eq!(response.cache_read_input_token_cost.unwrap().0, dec("0.5"));
    assert_eq!(response.cache_creation_token_cost.unwrap().0, dec("0.5"));
    assert_eq!(response.output_cost_per_token.unwrap().0, Decimal::ZERO);
}

#[test]
fn keeps_a_configured_cache_rate_when_the_model_is_unknown() {
    let response = helper_quote(
        r#"[{"model_name":"cache-priced","litellm_params":{"model":"self-hosted",
            "custom_llm_provider":"openai","input_cost_per_token":0.5,"output_cost_per_token":1.5,
            "cache_read_input_token_cost":0.25}}]"#,
        r#"{"model":"cache-priced","input_tokens":8,"output_tokens":4,
            "cache_read_input_tokens":2}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("9.5"));
    assert_eq!(response.input_cost_per_request.0, dec("3.5"));
    assert_eq!(response.output_cost_per_request.0, dec("6"));
    assert_eq!(response.cache_read_cost_per_request.0, dec("0.5"));
    assert_eq!(response.cache_read_input_token_cost.unwrap().0, dec("0.25"));
}

#[test]
fn falls_back_to_the_backend_cache_rates() {
    let response = helper_quote(
        r#"[{"model_name":"period-quote","litellm_params":{"model":"claude-haiku-4-5-20251001",
            "custom_llm_provider":"anthropic","input_cost_per_token":0.5,"output_cost_per_token":1.5}}]"#,
        r#"{"model":"period-quote","input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,
            "cache_creation_input_tokens":2,"num_requests_per_day":0,"num_requests_per_month":4}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("8.0000027"));
    assert_eq!(response.input_cost_per_request.0, dec("2.0000027"));
    assert_eq!(response.cache_read_cost_per_request.0, dec("0.0000002"));
    assert_eq!(response.cache_creation_cost.0, dec("0.0000025"));
    assert_eq!(
        response.cache_read_input_token_cost.unwrap().0,
        dec("0.0000001")
    );
    assert_eq!(
        response.cache_creation_token_cost.unwrap().0,
        dec("0.00000125")
    );
    assert_eq!(response.daily_cost, None);
    assert_eq!(response.monthly_cost.unwrap().0, dec("32.0000108"));
}

#[test]
fn prefers_the_declared_base_model() {
    let response = helper_quote(
        r#"[{"model_name":"alias-to-mini","litellm_params":{"model":"openai/renamed-mini"},
            "model_info":{"base_model":"gpt-4o-mini"}}]"#,
        r#"{"model":"alias-to-mini","input_tokens":1024,"output_tokens":256}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("0.0003072"));
}

#[test]
fn falls_through_an_empty_base_model() {
    let response = helper_quote(
        r#"[{"model_name":"empty-base","litellm_params":{"model":"openai/gpt-4o-mini"},
            "model_info":{"base_model":""}}]"#,
        r#"{"model":"empty-base","input_tokens":1024,"output_tokens":256}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("0.0003072"));
}

#[test]
fn answers_from_the_first_listed_deployment() {
    let response = helper_quote(
        r#"[{"model_name":"shared","litellm_params":{"model":"gpt-4o-mini"}},
            {"model_name":"shared","litellm_params":{"model":"gpt-4o"}}]"#,
        r#"{"model":"shared","input_tokens":1024,"output_tokens":256}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("0.0003072"));
}

#[test]
fn falls_back_to_the_deployment_provider() {
    let response = helper_quote(
        r#"[{"model_name":"self-hosted","litellm_params":{"model":"self-hosted",
            "custom_llm_provider":"openai","input_cost_per_token":0.000001,
            "output_cost_per_token":0.000002}}]"#,
        r#"{"model":"self-hosted","input_tokens":10,"output_tokens":5}"#,
    );
    assert_eq!(response.cost_per_request.0, dec("0.00002"));
    assert_eq!(response.provider.as_deref(), Some("openai"));
}

#[test]
fn rejects_a_deployment_record_without_a_model_name() {
    let rows: Vec<JsonObject> =
        serde_json::from_str(r#"[{"litellm_params":{"model":"gpt-4o"}}]"#).unwrap();
    let body: JsonValue =
        serde_json::from_str(r#"{"model":"x","input_tokens":1,"output_tokens":1}"#).unwrap();
    testing::quote(&rows, &body).unwrap_err();
}

#[test]
fn rejects_a_deployment_record_with_a_non_object_params() {
    let rows: Vec<JsonObject> =
        serde_json::from_str(r#"[{"model_name":"x","litellm_params":5}]"#).unwrap();
    let body: JsonValue =
        serde_json::from_str(r#"{"model":"x","input_tokens":1,"output_tokens":1}"#).unwrap();
    testing::quote(&rows, &body).unwrap_err();
}

#[test]
fn rejects_a_record_that_parses_as_no_deployment() {
    let rows: Vec<JsonObject> =
        serde_json::from_str(r#"[{"model_name":"x","model_info":{"id":true}}]"#).unwrap();
    assert!(DeploymentSet::from_records(&rows).is_err());
}

#[test]
fn rejects_a_priority_group_shadowed_by_a_model_name() {
    let owned = config(
        "general_settings:\n  master_key: oracle-master\nmodel_list:\n  - model_name: shared-name\n    litellm_params: {model: gpt-4o-mini}\nrouter_settings:\n  routing_groups:\n    - group_name: shared-name\n      models: [gpt-4o-mini]\n      routing_strategy: priority\n      model_priorities: {gpt-4o-mini: 1}\n",
    );
    assert!(DeploymentSet::from_config(&owned).is_err());
}

#[tokio::test]
async fn never_counts_tokens_of_its_own() {
    let counter = NeverCounts;
    assert_eq!(counter.count_messages("gpt-4o", &[]).await.unwrap(), 0);
    assert_eq!(counter.count_text("gpt-4o", "hello").await.unwrap(), 0);
}

#[tokio::test]
async fn answers_not_found_for_an_unpriceable_model() {
    let (status, body) = post(
        application(&config(AUTH)),
        r#"{"model":"no-such-model-xyz","input_tokens":10,"output_tokens":5}"#,
        Some("oracle-master"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains(
        "Could not calculate cost for model 'no-such-model-xyz' (resolved to 'no-such-model-xyz')"
    ));
}

#[tokio::test]
async fn answers_not_found_when_the_cost_service_is_unavailable() {
    let owned = config(AUTH);
    let mut state = state_for(&owned);
    Arc::get_mut(&mut state).unwrap().cost_service = Slot {
        service: Err(SlotError::Bundled(CatalogError::TooLarge {
            size: 2,
            limit: 1,
        })),
    };
    let app = routes::router(
        state,
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let (status, body) = post(
        app,
        r#"{"model":"gpt-4o","input_tokens":1,"output_tokens":1}"#,
        Some("oracle-master"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("the cost service is unavailable"));
}

#[tokio::test]
async fn selects_the_off_peak_prices_at_the_pinned_moment() {
    let owned = config(OPENROUTER);
    let deployments = DeploymentSet::from_config(&owned).unwrap();
    let asset = (BundledCatalog::default().asset)().unwrap();
    let service = testing::service_from(&asset).unwrap();
    let request: JsonValue = serde_json::from_str(
        r#"{"model":"openrouter-offpeak","input_tokens":1000,"output_tokens":500}"#,
    )
    .unwrap();
    let parsed = CostEstimateRequest::from_value(&request).unwrap();
    let off_peak = estimate(
        &service,
        Some(&deployments),
        &parsed,
        UNIX_EPOCH + Duration::from_hours(491_010),
    )
    .await
    .unwrap();
    assert_eq!(off_peak.cost_per_request.0, dec("0.00165"));
    assert_eq!(off_peak.input_cost_per_request.0, dec("0.00066"));
    assert_eq!(off_peak.input_cost_per_token.unwrap().0, dec("0.00000066"));
    assert_eq!(off_peak.output_cost_per_token.unwrap().0, dec("0.00000198"));
    let standard = estimate(
        &service,
        Some(&deployments),
        &parsed,
        UNIX_EPOCH + Duration::from_hours(490_994),
    )
    .await
    .unwrap();
    assert_eq!(standard.cost_per_request.0, dec("0.0033"));
    assert_eq!(standard.input_cost_per_token.unwrap().0, dec("0.00000132"));
    assert_eq!(standard.output_cost_per_token.unwrap().0, dec("0.00000396"));
}

#[rstest]
fn replays_the_cost_estimate_request_corpus(
    #[files("fuzz/corpus/cost_estimate_request/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_cost_estimate_request(&bytes));
}
