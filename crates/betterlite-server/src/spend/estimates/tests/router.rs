use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::spend::estimates::ports::{Clock as _, SystemClock};
use crate::spend::estimates::router::MAX_REQUEST_BYTES;

use super::support::{
    body_json, configured_router, cost_of, decimal, deployment, fake_reads, get_budgets, post_json,
    provider_config, state,
};

const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;

fn mini_state() -> Arc<crate::spend::estimates::router::EstimatesState> {
    state(Some(configured_router(
        vec![deployment("mini", r#"{"model":"openai/gpt-4o-mini"}"#)],
        AliasMap::new(),
        None,
        None,
    )))
}

fn error_field(body: &str, field: &str) -> serde_json::Value {
    body_json(body)["error"][field].clone()
}

#[tokio::test]
async fn the_calculate_route_answers_the_matched_models_cost() {
    let (status, body) = post_json(
        mini_state(),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cost_of(&body), decimal("0.0000015"));
}

#[tokio::test]
async fn the_calculate_route_answers_a_response_only_cost() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o-mini","choices":[],
        "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
    let (status, body) = post_json(
        state(None),
        &format!(r#"{{"completion_response":{response}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cost_of(&body), decimal("0.0000045"));
}

#[tokio::test]
async fn the_calculate_route_rejects_a_typed_field_with_422() {
    let (status, body) = post_json(state(None), r#"{"messages":"hello"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let detail = &body_json(&body)["detail"][0];
    assert_eq!(detail["type"], "list_type");
    assert_eq!(detail["loc"], serde_json::json!(["body", "messages"]));
}

#[tokio::test]
async fn the_calculate_route_rejects_malformed_json_with_422() {
    let (status, body) = post_json(state(None), "{").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(&body)["detail"][0]["type"], "json_invalid");
}

#[tokio::test]
async fn an_unknown_model_answers_invalid_request_error() {
    let (status, body) = post_json(
        state(None),
        &format!(r#"{{"model":"openai/not-a-model","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_field(&body, "type"), "invalid_request_error");
    assert_eq!(error_field(&body, "param"), "model");
    assert_eq!(error_field(&body, "code"), "400");
}

#[tokio::test]
async fn a_model_without_a_provider_answers_the_null_defaults() {
    let (status, body) = post_json(
        state(None),
        &format!(r#"{{"model":"not-a-model","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error_field(&body, "message")
            .as_str()
            .unwrap()
            .contains("LLM Provider NOT provided")
    );
    assert!(error_field(&body, "type").is_null());
    assert!(error_field(&body, "param").is_null());
    assert_eq!(error_field(&body, "code"), "400");
}

#[tokio::test]
async fn a_model_without_messages_answers_the_string_none_defaults() {
    let (status, body) = post_json(state(None), r#"{"model":"gpt-4o-mini"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error_field(&body, "message"),
        "Bad Request - messages must be provided if 'model' is provided"
    );
    assert_eq!(error_field(&body, "type"), "None");
    assert_eq!(error_field(&body, "param"), "None");
    assert_eq!(error_field(&body, "code"), "400");
}

#[tokio::test]
async fn neither_input_answers_the_string_none_defaults() {
    let (status, body) = post_json(state(None), "{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error_field(&body, "message"),
        "Bad Request - Either 'model' or 'completion_response' must be provided"
    );
    assert_eq!(error_field(&body, "type"), "None");
    assert_eq!(error_field(&body, "param"), "None");
    assert_eq!(error_field(&body, "code"), "400");
}

#[tokio::test]
async fn a_cost_failure_answers_a_500_with_none_defaults() {
    let state = state(Some(configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"openai/gpt-4o-mini","input_cost_per_token":0.001}"#,
        )],
        AliasMap::new(),
        None,
        None,
    )));
    let (status, body) = post_json(
        state,
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error_field(&body, "type"), "None");
    assert_eq!(error_field(&body, "param"), "None");
    assert_eq!(error_field(&body, "code"), "500");
}

#[tokio::test]
async fn the_budget_route_lists_every_configured_provider() {
    let mut reads = fake_reads(&[("openai", Some("25.5")), ("azure", None)]);
    reads
        .reset
        .insert("openai".to_owned(), "2026-03-01T12:00:60+00:00".to_owned());
    let router = configured_router(
        Vec::new(),
        AliasMap::new(),
        Some(provider_config(&[
            ("openai", Some("100"), Some("1d")),
            ("azure", None, Some("10d")),
        ])),
        Some(Arc::new(reads)),
    );
    let (status, body) = get_budgets(state(Some(router))).await;
    assert_eq!(status, StatusCode::OK);
    let value = body_json(&body);
    let openai = &value["providers"]["openai"];
    assert_eq!(openai["budget_limit"].as_f64(), Some(100.0));
    assert_eq!(openai["time_period"], "1d");
    assert_eq!(openai["spend"].as_f64(), Some(25.5));
    assert_eq!(openai["budget_reset_at"], "2026-03-01T12:00:60+00:00");
    let azure = &value["providers"]["azure"];
    assert!(azure["budget_limit"].is_null());
    assert_eq!(azure["time_period"], "10d");
    assert_eq!(azure["spend"].as_f64(), Some(0.0));
    assert!(azure["budget_reset_at"].is_null());
    assert!(body.find("\"openai\"").unwrap() < body.find("\"azure\"").unwrap());
}

#[tokio::test]
async fn a_budget_route_without_a_router_answers_its_500() {
    let (status, body) = get_budgets(state(None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error_field(&body, "message"),
        "{'error': 'No llm_router found'}"
    );
    assert_eq!(error_field(&body, "type"), "internal_server_error");
    assert!(error_field(&body, "param").is_null());
    assert_eq!(error_field(&body, "code"), "500");
}

#[tokio::test]
async fn a_budget_route_without_a_config_answers_its_500() {
    let router = configured_router(
        Vec::new(),
        AliasMap::new(),
        None,
        Some(Arc::new(fake_reads(&[]))),
    );
    let (status, body) = get_budgets(state(Some(router))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error_field(&body, "type"), "internal_server_error");
    assert!(
        error_field(&body, "message")
            .as_str()
            .unwrap()
            .starts_with("No provider budget config found.")
    );
}

#[tokio::test]
async fn a_budget_route_without_a_limiter_answers_its_500() {
    let router = configured_router(
        Vec::new(),
        AliasMap::new(),
        Some(provider_config(&[("openai", Some("100"), Some("1d"))])),
        None,
    );
    let (status, body) = get_budgets(state(Some(router))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error_field(&body, "message"),
        "No router budget logger found"
    );
}

#[tokio::test]
async fn a_budget_read_failure_answers_its_500() {
    let mut reads = fake_reads(&[("openai", Some("1"))]);
    reads
        .spend
        .insert("openai".to_owned(), super::support::SpendAnswer::Fail);
    let router = configured_router(
        Vec::new(),
        AliasMap::new(),
        Some(provider_config(&[("openai", Some("100"), Some("1d"))])),
        Some(Arc::new(reads)),
    );
    let (status, body) = get_budgets(state(Some(router))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error_field(&body, "type"), "internal_server_error");
    assert!(error_field(&body, "param").is_null());
    assert_eq!(error_field(&body, "code"), "500");
}

#[test]
fn the_system_clock_answers_the_current_time() {
    let difference = (SystemClock.now() - Utc::now()).num_seconds().abs();
    assert!(difference < 60);
}

#[tokio::test]
async fn the_calculate_route_accepts_a_body_near_the_two_mebibyte_limit() {
    let padding = "x".repeat(1_100_000);
    let body =
        format!(r#"{{"model":"mini","messages":[{{"role":"user","content":"{padding}"}}]}}"#);
    let (status, body) = post_json(mini_state(), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cost_of(&body), decimal("0.0000015"));
}

#[tokio::test]
async fn the_calculate_route_rejects_a_body_over_the_limit() {
    let body = "x".repeat(MAX_REQUEST_BYTES + 1);
    let (status, _body) = post_json(mini_state(), &body).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

// The per-provider amount is a JSON number, never a quoted string, matching the response model's float.
#[tokio::test]
async fn the_budget_route_keeps_the_amounts_numeric() {
    let router = configured_router(
        Vec::new(),
        AliasMap::new(),
        Some(provider_config(&[("openai", Some("0.5"), Some("1d"))])),
        Some(Arc::new(fake_reads(&[("openai", Some("0.25"))]))),
    );
    let (status, body) = get_budgets(state(Some(router))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&body)["providers"]["openai"]["spend"], 0.25);
    let amount = body_json(&body)["providers"]["openai"]["budget_limit"].clone();
    assert_eq!(
        Decimal::from_str_exact(&amount.to_string()).unwrap(),
        decimal("0.5")
    );
}
