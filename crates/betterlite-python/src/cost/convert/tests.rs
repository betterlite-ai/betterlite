use std::str::FromStr as _;

use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_core::routes::CallType;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use betterlite_models::billed_tokens::BilledTokenRates;

use crate::cost::billing::{CallRequest, CompletionRequest, ResponseWire};
use crate::cost::convert::{
    Amount, CostAnswer, RatesView, UsageAnswer, UsageRecord, encode, optional, optional_unit, parse,
};

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

#[test]
fn cost_request_reads_every_named_argument() {
    let text = r#"{
        "model": "gpt-4o",
        "prompt_tokens": 11,
        "completion_tokens": 7,
        "response_time_ms": 250.5,
        "custom_llm_provider": "openai",
        "region_name": "us-east-1",
        "prompt_characters": 40,
        "completion_characters": 12,
        "cache_creation_input_tokens": 3,
        "cache_read_input_tokens": 5,
        "custom_cost_per_token": {
            "input_cost_per_token": 0.1,
            "output_cost_per_token": 0.2,
            "cache_read_input_token_cost": 0.05
        },
        "custom_cost_per_second": 0.4,
        "number_of_queries": 2,
        "call_type": "embedding",
        "audio_seconds": 9.5,
        "service_tier": "flex",
        "data_residency": "eu",
        "vertex_location": "global",
        "request_model": "gpt-4o-mini"
    }"#;
    let request: CallRequest = parse(text).expect("the request parses");
    let input = request.input(None, None);
    assert_eq!(input.model, Some("gpt-4o"));
    assert_eq!(input.prompt_tokens, 11);
    assert_eq!(input.completion_tokens, 7);
    assert_eq!(input.response_time_ms, Some(dec("250.5")));
    assert_eq!(input.custom_llm_provider, Some("openai"));
    assert_eq!(input.region_name, Some("us-east-1"));
    assert_eq!(input.prompt_characters, Some(40));
    assert_eq!(input.completion_characters, Some(12));
    assert_eq!(input.cache_creation_input_tokens, Some(3));
    assert_eq!(input.cache_read_input_tokens, Some(5));
    let custom = input
        .custom_cost_per_token
        .expect("the custom rates arrive");
    assert_eq!(custom.input_cost_per_token, dec("0.1"));
    assert_eq!(custom.output_cost_per_token, dec("0.2"));
    assert_eq!(custom.cache_read_input_token_cost, Some(dec("0.05")));
    assert_eq!(custom.cache_creation_cost, None);
    assert_eq!(input.custom_cost_per_second, Some(dec("0.4")));
    assert_eq!(input.number_of_queries, Some(2));
    assert_eq!(input.call_type, CallType::Embedding);
    assert_eq!(input.audio_seconds, dec("9.5"));
    assert_eq!(input.service_tier, Some("flex"));
    assert_eq!(input.data_residency, Some("eu"));
    assert_eq!(input.vertex_location, Some("global"));
    assert_eq!(input.request_model, Some("gpt-4o-mini"));
    assert_eq!(input.usage, None);
    assert_eq!(input.rerank_billed_units, None);
}

#[test]
fn cost_request_defaults_the_call_type_and_the_empty_counts() {
    let request: CallRequest = parse("{}").expect("the empty request parses");
    let input = request.input(None, None);
    assert_eq!(input.model, None);
    assert_eq!(input.prompt_tokens, 0);
    assert_eq!(input.completion_tokens, 0);
    assert_eq!(input.response_time_ms, None);
    assert_eq!(input.call_type, CallType::Completion);
    assert_eq!(input.audio_seconds, Decimal::ZERO);
    assert_eq!(input.custom_cost_per_token, None);
}

#[test]
fn cost_request_rejects_a_field_it_does_not_read() {
    let error = parse::<CallRequest>(r#"{"not_a_cost_field": 1}"#)
        .err()
        .expect("an unknown field is refused");
    assert!(error.to_string().contains("not_a_cost_field"));
}

#[test]
fn cost_request_rejects_text_that_is_not_json() {
    let error = parse::<CallRequest>("not json")
        .err()
        .expect("malformed text is refused");
    assert!(error.to_string().contains("JSON"));
}

#[test]
fn cost_request_carries_the_usage_mapping_and_the_billed_units() {
    let request: CallRequest = parse(
        r#"{
            "model": "rerank-test",
            "usage": {"prompt_tokens": 3},
            "rerank_billed_units": {"search_units": 2}
        }"#,
    )
    .expect("the request parses");
    let usage = Usage {
        prompt_tokens: 3,
        completion_tokens: 0,
        total_tokens: 3,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    };
    let units = request
        .rerank_billed_units
        .as_ref()
        .expect("the units arrive")
        .billed();
    assert_eq!(units.search_units, Some(Some(2)));
    assert_eq!(units.total_tokens, None);
    let input = request.input(Some(&usage), Some(&units));
    assert_eq!(input.usage.map(|found| found.prompt_tokens), Some(3));
    assert_eq!(
        input.rerank_billed_units.map(|found| found.search_units),
        Some(Some(2))
    );
}

#[test]
fn cost_result_encodes_the_cost_pair_as_numbers() {
    let text = encode(&[Amount(dec("0.000010")), Amount(dec("0.25"))]).expect("the pair encodes");
    assert_eq!(text, "[0.000010,0.25]");
}

#[test]
fn cost_result_reads_a_chat_response_wire() {
    let response: ResponseWire = parse(
        r#"{
            "kind": "chat",
            "model": "gpt-4o",
            "usage": {"prompt_tokens": 4, "completion_tokens": 2},
            "service_tier": "flex",
            "hidden_params": {"custom_llm_provider": "openai"},
            "created": 1700000000,
            "ended": 1700000001
        }"#,
    )
    .expect("the response parses");
    assert_eq!(response.kind(), ResponseKind::Chat);
    assert!(matches!(response.billing_usage(), BillingUsage::Mapping(_)));
    assert_eq!(response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(response.service_tier.as_deref(), Some("flex"));
    assert!(response.hidden_params.is_some());
    assert_eq!(response.stamps().created, Some(dec("1700000000")));
    assert_eq!(response.stamps().ended, Some(dec("1700000001")));
}

#[test]
fn cost_result_treats_a_response_without_a_kind_as_other() {
    let response: ResponseWire = parse(r#"{"model": "gpt-4o"}"#).expect("the response parses");
    assert_eq!(response.kind(), ResponseKind::Other);
    assert!(matches!(response.billing_usage(), BillingUsage::Absent));
}

#[test]
fn cost_result_reads_every_completion_argument() {
    let text = r#"{
        "model": "gpt-4o",
        "prompt": "hello",
        "messages": [{"role": "user", "content": "hello"}],
        "completion": "world",
        "total_time": 1200.0,
        "call_type": "completion",
        "custom_llm_provider": "openai",
        "region_name": "us-east-1",
        "custom_cost_per_token": {"input_cost_per_token": 0.1, "output_cost_per_token": 0.2},
        "custom_cost_per_second": 0.4,
        "optional_params": {"temperature": 0},
        "custom_pricing": true,
        "base_model": "gpt-4o-2024",
        "router_model_id": "deployment-1",
        "service_tier": "flex",
        "data_residency": "eu",
        "vertex_location": "global",
        "request_model": "gpt-4o-mini",
        "response": {"kind": "chat", "usage": {"prompt_tokens": 1}}
    }"#;
    let request: CompletionRequest = parse(text).expect("the request parses");
    assert_eq!(request.model.as_deref(), Some("gpt-4o"));
    assert_eq!(request.prompt, "hello");
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.completion, "world");
    assert_eq!(request.total_time, Some(dec("1200.0")));
    assert_eq!(request.custom_pricing, Some(true));
    assert_eq!(request.base_model.as_deref(), Some("gpt-4o-2024"));
    assert_eq!(request.vertex_location.as_deref(), Some("global"));
    assert!(request.response.is_some());
    let charge = request.charge();
    assert_eq!(charge.total_time, Some(dec("1200.0")));
    assert_eq!(charge.custom_cost_per_second, Some(dec("0.4")));
    assert_eq!(charge.logging.model, Some("gpt-4o-mini"));
    assert!(charge.logging.params.is_none());
    assert!(charge.logging.timing.is_none());
    assert!(!charge.passthrough_image);
}

#[test]
fn cost_request_keeps_an_absent_amount_apart_from_a_zero_one() {
    assert_eq!(optional(None), None);
    assert_eq!(optional(Some(Amount(Decimal::ZERO))), Some(Decimal::ZERO));
    assert_eq!(optional_unit(None), None);
    assert_eq!(optional_unit(Some(0)), Some(Some(0)));
}

#[test]
fn cost_result_names_the_billed_rates_as_litellm_declares_them() {
    let rates = BilledTokenRates {
        input_cost_per_token: dec("0.1"),
        output_cost_per_token: dec("0.2"),
        cache_read_input_token_cost: dec("0.01"),
        audio_read: dec("0.02"),
        cache_write: dec("0.03"),
        cache_write_1h: dec("0.04"),
        reasoning: dec("0.05"),
    };
    let text = encode(&RatesView::from(rates)).expect("the rates encode");
    assert_eq!(
        text,
        r#"{"input_cost_per_token":0.1,"output_cost_per_token":0.2,"cache_read_input_token_cost":0.01,"cache_read_input_audio_token_cost":0.02,"cache_creation_input_token_cost":0.03,"cache_creation_input_token_cost_above_1hr":0.04,"output_cost_per_reasoning_token":0.05}"#
    );
}

#[test]
fn cost_result_writes_the_usage_record_and_the_cost_answer() {
    let usage = Usage {
        prompt_tokens: 4,
        completion_tokens: 2,
        total_tokens: 6,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: Some(dec("0.5")),
        extras: IndexMap::new(),
    };
    let mut extra: IndexMap<String, Json> = IndexMap::new();
    extra.insert(
        "character_count".to_owned(),
        Json::Number(serde_json::Number::from(12)),
    );
    let usage = Usage {
        extras: extra,
        ..usage
    };
    let answer = UsageAnswer {
        usage: Some(UsageRecord::from(&usage)),
    };
    let text = encode(&answer).expect("the usage answer encodes");
    assert_eq!(
        text,
        r#"{"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6,"prompt_tokens_details":null,"completion_tokens_details":null,"server_tool_use":null,"cost":0.5,"character_count":12}}"#
    );
    let empty = UsageAnswer { usage: None };
    assert_eq!(encode(&empty).unwrap(), r#"{"usage":null}"#);
    let answer = CostAnswer {
        cost: Amount(dec("0.02")),
        breakdown: None,
        billed_token_rates: None,
    };
    assert_eq!(
        encode(&answer).unwrap(),
        r#"{"cost":0.02,"breakdown":null,"billed_token_rates":null}"#
    );
}
