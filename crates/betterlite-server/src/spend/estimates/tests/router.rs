use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use chrono::Utc;

use crate::key_fields::values::Amount;
use crate::spend::estimates::ports::{Clock as _, SystemClock};
use crate::spend::estimates::router::MAX_REQUEST_BYTES;

use super::support::{
    budgets_of, configured_router, cost_of, decimal, deployment, error_of, fake_reads, get_budgets,
    post_json, provider_config, state, text, validation_detail,
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
    let detail = validation_detail(&body);
    assert_eq!(detail.kind, "list_type");
    assert_eq!(detail.loc, vec!["body".to_owned(), "messages".to_owned()]);
}

#[tokio::test]
async fn the_calculate_route_rejects_malformed_json_with_422() {
    let (status, body) = post_json(state(None), "{").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(validation_detail(&body).kind, "json_invalid");
}

#[tokio::test]
async fn an_unknown_model_answers_invalid_request_error() {
    let (status, body) = post_json(
        state(None),
        &format!(r#"{{"model":"openai/not-a-model","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = error_of(&body);
    assert_eq!(error.kind, text("invalid_request_error"));
    assert_eq!(error.param, text("model"));
    assert_eq!(error.code, text("400"));
}

#[tokio::test]
async fn a_model_without_a_provider_answers_the_null_defaults() {
    let (status, body) = post_json(
        state(None),
        &format!(r#"{{"model":"not-a-model","messages":{MESSAGES}}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = error_of(&body);
    assert!(error.message.contains("LLM Provider NOT provided"));
    assert_eq!(error.kind, Json::Null);
    assert_eq!(error.param, Json::Null);
    assert_eq!(error.code, text("400"));
}

#[tokio::test]
async fn a_model_without_messages_answers_the_string_none_defaults() {
    let (status, body) = post_json(state(None), r#"{"model":"gpt-4o-mini"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = error_of(&body);
    assert_eq!(
        error.message,
        "Bad Request - messages must be provided if 'model' is provided"
    );
    assert_eq!(error.kind, text("None"));
    assert_eq!(error.param, text("None"));
    assert_eq!(error.code, text("400"));
}

#[tokio::test]
async fn neither_input_answers_the_string_none_defaults() {
    let (status, body) = post_json(state(None), "{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = error_of(&body);
    assert_eq!(
        error.message,
        "Bad Request - Either 'model' or 'completion_response' must be provided"
    );
    assert_eq!(error.kind, text("None"));
    assert_eq!(error.param, text("None"));
    assert_eq!(error.code, text("400"));
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
    let error = error_of(&body);
    assert_eq!(error.kind, text("None"));
    assert_eq!(error.param, text("None"));
    assert_eq!(error.code, text("500"));
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
    let providers = budgets_of(&body);
    let openai = &providers["openai"];
    assert_eq!(openai.budget_limit, Some(Amount(decimal("100"))));
    assert_eq!(openai.time_period.as_deref(), Some("1d"));
    assert_eq!(openai.spend, Amount(decimal("25.5")));
    assert_eq!(
        openai.budget_reset_at.as_deref(),
        Some("2026-03-01T12:00:60+00:00")
    );
    let azure = &providers["azure"];
    assert_eq!(azure.budget_limit, None);
    assert_eq!(azure.time_period.as_deref(), Some("10d"));
    assert_eq!(azure.spend, Amount(decimal("0")));
    assert_eq!(azure.budget_reset_at, None);
    assert!(body.find("\"openai\"").unwrap() < body.find("\"azure\"").unwrap());
}

#[tokio::test]
async fn a_budget_route_without_a_router_answers_its_500() {
    let (status, body) = get_budgets(state(None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = error_of(&body);
    assert_eq!(error.message, "{'error': 'No llm_router found'}");
    assert_eq!(error.kind, text("internal_server_error"));
    assert_eq!(error.param, Json::Null);
    assert_eq!(error.code, text("500"));
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
    let error = error_of(&body);
    assert_eq!(error.kind, text("internal_server_error"));
    assert!(
        error
            .message
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
    assert_eq!(error_of(&body).message, "No router budget logger found");
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
    let error = error_of(&body);
    assert_eq!(error.kind, text("internal_server_error"));
    assert_eq!(error.param, Json::Null);
    assert_eq!(error.code, text("500"));
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
    assert!(body.contains(r#""budget_limit":0.5"#));
    assert!(body.contains(r#""spend":0.25"#));
    assert_eq!(budgets_of(&body)["openai"].spend, Amount(decimal("0.25")));
}
