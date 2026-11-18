use std::sync::Arc;

use betterlite_redact::secret::Redactor;
use rust_decimal::Decimal;

use crate::streamed_response_assembly::records::Chunk;

use super::support::{ProviderSeen, chunks, priced_logging};
use super::{Harness, chat_of, cost_stream, hidden_stream, plain_stream, value};

pub(super) async fn reported_cost_precedence(redactor: &Arc<Redactor>) {
    reported_cost_results(redactor).await;
    reported_cost_xai(redactor).await;
    reported_cost_logging_without_reported(redactor).await;
}

// One stream, what the completion_cost fixture does, and what the response carries afterwards.
struct Case {
    stream: Vec<Chunk>,
    priced: Option<Decimal>,
    completion_error: bool,
    per_token_error: bool,
    reported: Option<Decimal>,
    response_cost: Option<&'static str>,
    provider: ProviderSeen,
}

// A call to completion_cost with no hidden provider and no price, so only the stream decides.
fn base_case(stream: Vec<Chunk>) -> Case {
    Case {
        stream,
        priced: None,
        completion_error: false,
        per_token_error: false,
        reported: None,
        response_cost: None,
        provider: ProviderSeen::Named(None),
    }
}

fn value_cases() -> [Case; 4] {
    [
        Case {
            reported: Some(Decimal::new(25, 2)),
            response_cost: Some("0.25"),
            provider: ProviderSeen::NotCalled,
            ..base_case(cost_stream(Some("0.25")))
        },
        Case {
            reported: Some(Decimal::ONE),
            response_cost: Some("1"),
            provider: ProviderSeen::NotCalled,
            ..base_case(cost_stream(Some("true")))
        },
        Case {
            priced: Some(Decimal::new(5, 1)),
            response_cost: Some("0.5"),
            provider: ProviderSeen::Named(Some("openai".to_owned())),
            ..base_case(hidden_stream(r#"{"custom_llm_provider": "openai"}"#))
        },
        Case {
            response_cost: Some("0.5"),
            ..base_case(hidden_stream(r#"{"response_cost": 0.5}"#))
        },
    ]
}

// A failing completion_cost falls back to the model-map prices, unless they fail or the model is empty.
fn fallback_cases() -> [Case; 3] {
    let empty_model = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "",
         "choices": [{"index": 0, "delta": {"content": "A"}}]}
    ]"#,
    );
    [
        Case {
            completion_error: true,
            response_cost: Some("0.0003"),
            ..base_case(plain_stream())
        },
        Case {
            completion_error: true,
            per_token_error: true,
            ..base_case(plain_stream())
        },
        Case {
            completion_error: true,
            ..base_case(empty_model)
        },
    ]
}

pub(super) async fn reported_cost_results(redactor: &Arc<Redactor>) {
    for scenario in value_cases().into_iter().chain(fallback_cases()) {
        let mut harness = Harness::new(Some(scenario.stream), redactor);
        harness.messages = true;
        harness.counter.response_text = 1;
        harness.costs.completion = scenario.priced;
        harness.costs.completion_error = scenario.completion_error;
        harness.costs.per_token_error = scenario.per_token_error;
        harness.costs.prompt = Some(Decimal::new(1, 4));
        harness.costs.completion_tokens = Some(Decimal::new(2, 4));
        let response = chat_of(&harness).await;
        assert_eq!(response.usage.cost, scenario.reported);
        assert_eq!(
            response.hidden_params.get("response_cost"),
            scenario.response_cost.map(value).as_ref()
        );
        assert_eq!(harness.costs.seen_provider(), scenario.provider);
    }
}

pub(super) async fn reported_cost_xai(redactor: &Arc<Redactor>) {
    let logging = priced_logging("xai", None);
    let mut harness = Harness::new(Some(cost_stream(Some("0.25"))), redactor);
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 0);
    assert_eq!(harness.costs.completion_calls(), 0);
}

pub(super) async fn reported_cost_logging_without_reported(redactor: &Arc<Redactor>) {
    let logging = priced_logging("openai", None);
    let mut harness = Harness::new(Some(plain_stream()), redactor);
    harness.messages = true;
    harness.counter.response_text = 1;
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(harness.costs.completion_calls(), 0);
    assert_eq!(logging.calls(), 1);
}
