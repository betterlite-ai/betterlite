#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget as _};
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CachedTokensDetails, PromptTokensDetails};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{Capture, prompt_extras, usage_record};

#[rstest]
#[case::plain(None)]
#[case::input_only(Some("input_tokens"))]
#[case::output_only(Some("output_tokens"))]
fn cost_extra_support_preserves_chat_usage(#[case] response_key: Option<&str>) {
    let mut mapping: IndexMap<String, Json> = serde_json::from_str(
        r#"{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14,
            "prompt_tokens_details":{"character_count":123,"cached_tokens":2},
            "completion_tokens_details":{"accepted_prediction_tokens":4,
                "rejected_prediction_tokens":1}}"#,
    )
    .unwrap();
    if let Some(key) = response_key {
        mapping.insert(key.to_owned(), Json::Number(Number::from(99_i64)));
    }
    let collected = usage_record(&mapping).unwrap();
    assert_eq!(collected.prompt_tokens, 10);
    assert_eq!(collected.completion_tokens, 4);
    assert_eq!(collected.total_tokens, 14);
    assert_eq!(
        collected.prompt_tokens_details.map(Json::Object),
        mapping.get("prompt_tokens_details").cloned()
    );
    assert_eq!(
        collected.completion_tokens_details.map(Json::Object),
        mapping.get("completion_tokens_details").cloned()
    );
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn prompt_fixture() -> PromptTokensDetails {
    PromptTokensDetails {
        text_tokens: Some(1),
        audio_tokens: Some(2),
        image_tokens: Some(3),
        video_tokens: Some(4),
        web_search_requests: Some(5),
        google_maps_grounding_requests: Some(6),
        tool_use_tokens: Some(7),
        character_count: Some(8),
        image_count: Some(9),
        video_length_seconds: Some(dec("1.5")),
        audio_length_seconds: Some(dec("2.5")),
        query_count: Some(10),
        cache_write_tokens: Some(11),
        cache_creation_tokens: Some(12),
        cached_tokens: Some(13),
        cached_tokens_details: Some(CachedTokensDetails {
            text_tokens: Some(14),
            audio_tokens: Some(15),
            image_tokens: Some(16),
        }),
    }
}

#[test]
fn cost_extra_support_prompt_extras_maps_every_prompt_detail() {
    let details = prompt_fixture();
    let extras = prompt_extras(Some(&details));
    assert_eq!(extras.cache_write_tokens, Some(11));
    assert_eq!(extras.cache_creation_tokens, Some(12));
    assert!(extras.cache_creation_token_details.is_none());
    assert_eq!(
        extras.cached_tokens_details.map(|cached| (
            cached.text_tokens,
            cached.audio_tokens,
            cached.image_tokens
        )),
        Some((Some(14), Some(15), Some(16)))
    );
    assert_eq!(extras.character_count, Some(8));
    assert_eq!(extras.image_count, Some(9));
    assert_eq!(extras.video_length_seconds, Some(dec("1.5")));
    assert_eq!(extras.audio_length_seconds, Some(dec("2.5")));
    assert_eq!(extras.query_count, Some(10));

    let absent = prompt_extras(None);
    assert!(absent.cached_tokens_details.is_none());
    assert!(absent.cache_write_tokens.is_none());
}

// Every cost a stream reports lands in the captured breakdown; an empty extra map becomes no map at all.
fn breakdown_input(additional: Option<BTreeMap<String, Decimal>>) -> CostBreakdownInput {
    CostBreakdownInput {
        input_cost: dec("0.001"),
        output_cost: dec("0.002"),
        total_cost: dec("0.003"),
        tool_usage_cost: dec("0.0004"),
        additional_costs: additional,
        original_cost: Some(dec("0.003")),
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        cache_read_cost: Some(dec("0.0005")),
        cache_creation_cost: Some(dec("0.0006")),
        reasoning_cost: Some(dec("0.0007")),
        service_tier: Some("default".to_owned()),
        data_residency: Some("eu".to_owned()),
        vertex_location: Some("us-central1".to_owned()),
        billed_token_rates: None,
    }
}

#[test]
fn cost_extra_support_capture_keeps_the_breakdown_and_drops_an_empty_extra_map() {
    let mut extra = BTreeMap::new();
    extra.insert("web_search".to_owned(), dec("0.01"));
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&breakdown_input(Some(extra.clone())))
        .expect("the capture accepts a breakdown");
    let breakdown = capture.breakdown.expect("the breakdown is kept");
    assert_eq!(breakdown.input_cost, Some(dec("0.001")));
    assert_eq!(breakdown.output_cost, Some(dec("0.002")));
    assert_eq!(breakdown.total_cost, Some(dec("0.003")));
    assert_eq!(breakdown.tool_usage_cost, Some(dec("0.0004")));
    assert_eq!(breakdown.cache_read_cost, Some(dec("0.0005")));
    assert_eq!(breakdown.cache_creation_cost, Some(dec("0.0006")));
    assert_eq!(breakdown.reasoning_cost, Some(dec("0.0007")));
    assert_eq!(breakdown.original_cost, Some(dec("0.003")));
    assert_eq!(breakdown.service_tier.as_deref(), Some("default"));
    assert_eq!(breakdown.data_residency.as_deref(), Some("eu"));
    assert_eq!(breakdown.vertex_location.as_deref(), Some("us-central1"));
    assert_eq!(breakdown.additional_costs, Some(extra));

    let mut empty = Capture::default();
    empty
        .set_cost_breakdown(&breakdown_input(Some(BTreeMap::new())))
        .expect("an empty extra map still captures");
    let dropped = empty.breakdown.expect("the empty breakdown is kept");
    assert!(dropped.additional_costs.is_none());
}
