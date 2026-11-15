use rust_decimal::Decimal;

use crate::streamed_response_assembly::records::Chunk;

use super::support::{FixtureLogging, ProviderSeen, chunks};
use super::{Harness, chat_of, cost_stream, hidden_stream, plain_stream, value};

pub(super) async fn reported_cost_precedence() {
    reported_cost_values().await;
    reported_cost_xai().await;
    reported_cost_logging_without_reported().await;
    reported_cost_fallback().await;
}

pub(super) async fn reported_cost_values() {
    for (stream, priced, reported, response_cost, provider) in value_cases() {
        let mut harness = Harness::new(Some(stream));
        harness.costs.completion = priced;
        let response = chat_of(&harness).await;
        assert_eq!(response.usage.cost, reported, "{response_cost}");
        assert_eq!(
            response.hidden_params.get("response_cost"),
            Some(&value(response_cost))
        );
        assert_eq!(harness.costs.seen_provider(), provider);
    }
}

type ValueCase = (
    Vec<Chunk>,
    Option<Decimal>,
    Option<Decimal>,
    &'static str,
    ProviderSeen,
);

// Each case is a stream, the price completion_cost answers with, the usage cost, the response cost and the provider seen.
fn value_cases() -> [ValueCase; 4] {
    [
        (
            cost_stream(Some("0.25")),
            None,
            Some(Decimal::new(25, 2)),
            "0.25",
            ProviderSeen::NotCalled,
        ),
        (
            cost_stream(Some("true")),
            None,
            Some(Decimal::ONE),
            "1",
            ProviderSeen::NotCalled,
        ),
        (
            hidden_stream(r#"{"custom_llm_provider": "openai"}"#),
            Some(Decimal::new(5, 1)),
            None,
            "0.5",
            ProviderSeen::Named(Some("openai".to_owned())),
        ),
        (
            hidden_stream(r#"{"response_cost": 0.5}"#),
            None,
            None,
            "0.5",
            ProviderSeen::Named(None),
        ),
    ]
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

// A failing completion_cost falls back to the model-map prices, unless they fail or the model is empty.
pub(super) async fn reported_cost_fallback() {
    let empty_model = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "",
         "choices": [{"index": 0, "delta": {"content": "A"}}]}
    ]"#,
    );
    let cases = [
        (plain_stream(), false, Some("0.0003")),
        (plain_stream(), true, None),
        (empty_model, false, None),
    ];
    for (stream, per_token_error, expected) in cases {
        let mut harness = Harness::new(Some(stream));
        harness.messages = true;
        harness.counter.response_text = 1;
        harness.costs.completion_error = true;
        harness.costs.per_token_error = per_token_error;
        harness.costs.prompt = Some(Decimal::new(1, 4));
        harness.costs.completion_tokens = Some(Decimal::new(2, 4));
        let response = chat_of(&harness).await;
        assert_eq!(harness.costs.completion_calls(), 1);
        assert_eq!(
            response.hidden_params.get("response_cost"),
            expected.map(value).as_ref()
        );
    }
}
