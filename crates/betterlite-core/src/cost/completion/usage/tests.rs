#![cfg(test)]

use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage as PricingUsage, UsageNumber,
};
use indexmap::IndexMap;
use serde_json::Number;

use super::{logging_usage, pricing_usage};
use crate::cost::completion::tests::rig::dec;

fn count(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn seconds(text: &str) -> Json {
    Json::Number(text.parse().expect("the fixture is a number"))
}

#[test]
fn the_logging_usage_keeps_only_the_counters_a_partition_set() {
    let usage = PricingUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: Some(3),
            audio_length_seconds: Some(dec("1.5")),
            video_length_seconds: Some(dec("2.5")),
            ..PromptTokensDetails::default()
        }),
        completion_tokens_details: Some(CompletionTokensDetails {
            reasoning_tokens: Some(2),
            ..CompletionTokensDetails::default()
        }),
        ..PricingUsage::default()
    };
    let projected = logging_usage(&usage).unwrap();
    assert_eq!(
        (
            projected.prompt_tokens,
            projected.completion_tokens,
            projected.total_tokens
        ),
        (10, 5, 15)
    );
    let prompt = projected.prompt_tokens_details.unwrap();
    assert_eq!(prompt.len(), 3);
    assert_eq!(prompt.get("cached_tokens"), Some(&count(3)));
    assert_eq!(prompt.get("audio_length_seconds"), Some(&seconds("1.5")));
    assert_eq!(prompt.get("video_length_seconds"), Some(&seconds("2.5")));
    let completion = projected.completion_tokens_details.unwrap();
    assert_eq!(completion.len(), 1);
    assert_eq!(completion.get("reasoning_tokens"), Some(&count(2)));
}

#[test]
fn a_length_alone_still_opens_the_prompt_details() {
    let usage = PricingUsage {
        prompt_tokens_details: Some(PromptTokensDetails {
            audio_length_seconds: Some(dec("4")),
            ..PromptTokensDetails::default()
        }),
        ..PricingUsage::default()
    };
    let prompt = logging_usage(&usage)
        .unwrap()
        .prompt_tokens_details
        .unwrap();
    assert_eq!(prompt.len(), 1);
    assert_eq!(prompt.get("audio_length_seconds"), Some(&seconds("4")));
}

#[test]
fn empty_details_project_to_nothing() {
    let usage = PricingUsage {
        prompt_tokens_details: Some(PromptTokensDetails::default()),
        completion_tokens_details: Some(CompletionTokensDetails::default()),
        ..PricingUsage::default()
    };
    let projected = logging_usage(&usage).unwrap();
    assert!(projected.prompt_tokens_details.is_none());
    assert!(projected.completion_tokens_details.is_none());
}

#[test]
fn only_the_cost_extension_becomes_the_stated_cost() {
    let mut usage = PricingUsage::default();
    usage
        .numeric_extras
        .insert("cost".to_owned(), UsageNumber::Decimal(dec("0.5")));
    usage
        .numeric_extras
        .insert("other".to_owned(), UsageNumber::Decimal(dec("0.25")));
    usage
        .numeric_extras
        .insert("whole".to_owned(), UsageNumber::Integer(4));
    usage
        .numeric_extras
        .insert("flag".to_owned(), UsageNumber::Boolean(true));
    let projected = logging_usage(&usage).unwrap();
    assert_eq!(projected.cost, Some(dec("0.5")));
    assert_eq!(projected.extras.len(), 3);
    assert_eq!(projected.extras.get("other"), Some(&seconds("0.25")));
    assert_eq!(projected.extras.get("whole"), Some(&count(4)));
    assert_eq!(projected.extras.get("flag"), Some(&Json::Bool(true)));
}

#[test]
fn the_pricing_usage_reads_the_counts_and_the_stated_cost() {
    let usage = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: Some(IndexMap::from([("cached_tokens".to_owned(), count(3))])),
        completion_tokens_details: None,
        server_tool_use: None,
        cost: Some(dec("0.5")),
        extras: IndexMap::new(),
    };
    let converted = pricing_usage(&usage).unwrap();
    assert_eq!(
        (
            converted.prompt_tokens,
            converted.completion_tokens,
            converted.total_tokens
        ),
        (10, 5, 15)
    );
    assert_eq!(
        converted.prompt_tokens_details.unwrap().cached_tokens,
        Some(3)
    );
    assert!(converted.completion_tokens_details.is_none());
    assert_eq!(
        converted.numeric_extras.get("cost"),
        Some(&UsageNumber::Decimal(dec("0.5")))
    );
}

#[test]
fn an_extension_keeps_its_kind_and_a_private_one_is_dropped() {
    let usage = Usage {
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::from([
            ("whole".to_owned(), count(4)),
            ("flag".to_owned(), Json::Bool(true)),
            ("ratio".to_owned(), seconds("0.25")),
            ("_hidden".to_owned(), count(9)),
            ("label".to_owned(), Json::Text("x".to_owned())),
        ]),
    };
    let converted = pricing_usage(&usage).unwrap();
    assert_eq!(
        converted.numeric_extras.get("whole"),
        Some(&UsageNumber::Integer(4))
    );
    assert_eq!(
        converted.numeric_extras.get("flag"),
        Some(&UsageNumber::Boolean(true))
    );
    assert_eq!(
        converted.numeric_extras.get("ratio"),
        Some(&UsageNumber::Decimal(dec("0.25")))
    );
    assert_eq!(converted.numeric_extras.len(), 3);
}
