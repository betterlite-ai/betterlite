#![cfg(test)]

use super::*;

#[test]
fn estimate_usage_composes_input_and_output_counts() {
    let tokenization = FixedTokenization::working();
    let request_input = json(r#""hello""#);
    let responses_api_request = json(r#"{"model":"gpt-4o","input":"hello"}"#);
    let request = EstimateRequest {
        model: "gpt-4o",
        request_input: &request_input,
        responses_api_request: &responses_api_request,
        generated_text: "generated",
    };
    let estimated = estimate_usage_from_text(&tokenization, &request).unwrap();
    assert_eq!(estimated.input_tokens, 3);
    assert_eq!(estimated.output_tokens, 4);
    assert_eq!(estimated.total_tokens, 7);
    assert!(estimated.input_tokens_details.is_none());
    assert!(estimated.output_tokens_details.is_none());
    assert!(estimated.cost.is_none());
    assert!(estimated.extra.is_empty());
    assert_eq!(
        tokenization.transforms(),
        [(request_input, responses_api_request)]
    );
    let counts = tokenization.counts();
    assert_eq!(counts.len(), 2);
    assert_eq!(counts[0].model, "gpt-4o");
    assert_eq!(
        counts[0].messages,
        Some(vec![json(r#"{"role":"user","content":"hello"}"#)])
    );
    assert!(counts[0].text.is_none());
    assert!(!counts[0].count_response_tokens);
    assert!(counts[1].messages.is_none());
    assert_eq!(counts[1].text.as_deref(), Some("generated"));
    assert!(counts[1].count_response_tokens);
}

#[test]
fn estimate_usage_safely_returns_none_and_logs() {
    let mut tokenization = FixedTokenization::working();
    tokenization.fails = true;
    let request_input = json(r#""hello""#);
    let responses_api_request = json("{}");
    let request = EstimateRequest {
        model: "gpt-4o",
        request_input: &request_input,
        responses_api_request: &responses_api_request,
        generated_text: "generated",
    };
    let (estimated, logs) = capture(|| estimate_usage_safely(&tokenization, &request));
    assert!(estimated.is_none());
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0]
            .contains("Could not estimate usage from stream text, billing $0: cost lookup failed")
    );
}

#[test]
fn estimate_usage_safely_returns_usage() {
    let tokenization = FixedTokenization::working();
    let request_input = json(r#""hello""#);
    let responses_api_request = json("{}");
    let request = EstimateRequest {
        model: "gpt-4o",
        request_input: &request_input,
        responses_api_request: &responses_api_request,
        generated_text: "generated",
    };
    let (estimated, logs) = capture(|| estimate_usage_safely(&tokenization, &request));
    assert_eq!(estimated.unwrap().total_tokens, 7);
    assert!(logs.is_empty());
}
