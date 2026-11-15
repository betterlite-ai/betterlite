use rust_decimal::Decimal;

use super::support::{FixtureLogging, ProviderSeen, chunks};
use super::{Harness, chat_of, cost_stream, hidden_stream, plain_stream, value};

pub(super) async fn reported_cost_precedence() {
    reported_cost_reported().await;
    reported_cost_bool().await;
    reported_cost_xai().await;
    reported_cost_logging_without_reported().await;
    reported_cost_provider_hint().await;
    reported_cost_fallback().await;
    reported_cost_fallback_failure().await;
    reported_cost_preserved().await;
}

pub(super) async fn reported_cost_reported() {
    let harness = Harness::new(Some(cost_stream(Some("0.25"))));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.25"))
    );
    assert_eq!(harness.costs.completion_calls(), 0);
}

pub(super) async fn reported_cost_bool() {
    let harness = Harness::new(Some(cost_stream(Some("true"))));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::ONE));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("1"))
    );
}

pub(super) async fn reported_cost_xai() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("xai");
    let mut harness = Harness::new(Some(cost_stream(Some("0.25"))));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 0);
    assert_eq!(harness.costs.completion_calls(), 0);
}

pub(super) async fn reported_cost_logging_without_reported() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(harness.costs.completion_calls(), 0);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn reported_cost_provider_hint() {
    let mut harness = Harness::new(Some(hidden_stream(r#"{"custom_llm_provider": "openai"}"#)));
    harness.costs.completion = Some(Decimal::new(5, 1));
    let response = chat_of(&harness).await;
    assert_eq!(
        harness.costs.seen_provider(),
        ProviderSeen::Named(Some("openai".to_owned()))
    );
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.5"))
    );
}

pub(super) async fn reported_cost_fallback() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.prompt = Some(Decimal::new(1, 4));
    harness.costs.completion_tokens = Some(Decimal::new(2, 4));
    let response = chat_of(&harness).await;
    assert_eq!(harness.costs.completion_calls(), 1);
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.0003"))
    );
}

pub(super) async fn reported_cost_fallback_failure() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.per_token_error = true;
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
    let empty_model = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "",
         "choices": [{"index": 0, "delta": {"content": "A"}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(empty_model));
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.costs.completion_error = true;
    harness.costs.prompt = Some(Decimal::new(1, 4));
    harness.costs.completion_tokens = Some(Decimal::new(2, 4));
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
}

pub(super) async fn reported_cost_preserved() {
    let harness = Harness::new(Some(hidden_stream(r#"{"response_cost": 0.5}"#)));
    let response = chat_of(&harness).await;
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.5"))
    );
}
