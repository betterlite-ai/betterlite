#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde::Deserialize;

use crate::usage::error::UsageError;
use crate::usage::{
    AnthropicUsage, CacheCreationTokenDetails, ReasoningTokenCounter, ServerToolUse,
    calculate_usage, is_anthropic_usage_object, thinking_tokens_from_usage,
};

#[derive(Debug, thiserror::Error)]
#[error("the test counter failed")]
struct CounterFailed;

struct FixedCounter(usize);

impl ReasoningTokenCounter for FixedCounter {
    type Failure = CounterFailed;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(self.0)
    }
}

struct FailingCounter;

impl ReasoningTokenCounter for FailingCounter {
    type Failure = CounterFailed;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Err(CounterFailed)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    usage: Json,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    completion_response: Option<Json>,
    #[serde(default)]
    speed: Option<String>,
    #[serde(default)]
    estimate: Option<usize>,
}

#[derive(Clone, Copy)]
enum Scenario {
    CacheReadWriteTotal,
    FiveMinuteOneHourBreakdown,
    IterationUsageSum,
    PartialIterationDetail,
    ReportedThinkingCapped,
    EstimatedThinkingLocal,
    SignedThinkingWithoutCount,
    ToolSearchCountFallback,
    SpeedTierGeography,
}

fn probe(source: &str) -> Probe {
    serde_json::from_str(source).unwrap()
}

fn project(source: &str) -> Result<AnthropicUsage, UsageError<CounterFailed>> {
    let probe = probe(source);
    calculate_usage(
        &probe.usage,
        probe.reasoning_content.as_deref(),
        probe.completion_response.as_ref(),
        probe.speed.as_deref(),
        &FixedCounter(probe.estimate.unwrap_or(0)),
    )
}

fn as_json(usage: &AnthropicUsage) -> Json {
    serde_json::from_str(&serde_json::to_string(usage).unwrap()).unwrap()
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::cache_read_write_total(
    Scenario::CacheReadWriteTotal,
    r#"{"usage": {"input_tokens": 10.9, "output_tokens": true, "cache_creation_input_tokens": 3, "cache_read_input_tokens": 2}}"#,
    r#"{
        "prompt_tokens": 15,
        "completion_tokens": 1,
        "total_tokens": 16,
        "prompt_tokens_details": {
            "text_tokens": 10,
            "cached_tokens": 2,
            "cache_creation_tokens": 3,
            "cache_write_tokens": 3,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 1,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 3,
        "cache_read_input_tokens": 2,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::five_minute_one_hour_breakdown(
    Scenario::FiveMinuteOneHourBreakdown,
    r#"{"usage": {"input_tokens": 10, "output_tokens": 5, "cache_creation": {"ephemeral_5m_input_tokens": 7, "ephemeral_1h_input_tokens": 1}}}"#,
    r#"{
        "prompt_tokens": 10,
        "completion_tokens": 5,
        "total_tokens": 15,
        "prompt_tokens_details": {
            "text_tokens": 10,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null,
            "cache_creation_token_details": {"ephemeral_5m_input_tokens": 7, "ephemeral_1h_input_tokens": 1}
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 5,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::iteration_usage_sum(
    Scenario::IterationUsageSum,
    r#"{"usage": {"input_tokens": 100, "output_tokens": 100, "cache_creation_input_tokens": 9, "cache_read_input_tokens": 8, "iterations": [{"input_tokens": 3, "output_tokens": 4, "cache_creation_input_tokens": 1, "cache_read_input_tokens": 2}, {"input_tokens": 10, "output_tokens": 20}]}}"#,
    r#"{
        "prompt_tokens": 16,
        "completion_tokens": 24,
        "total_tokens": 40,
        "prompt_tokens_details": {
            "text_tokens": 13,
            "cached_tokens": 2,
            "cache_creation_tokens": 1,
            "cache_write_tokens": 1,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 24,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 1,
        "cache_read_input_tokens": 2,
        "iterations": [
            {"input_tokens": 3, "output_tokens": 4, "cache_creation_input_tokens": 1, "cache_read_input_tokens": 2},
            {"input_tokens": 10, "output_tokens": 20}
        ],
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::partial_iteration_detail(
    Scenario::PartialIterationDetail,
    r#"{"usage": {"input_tokens": 100, "output_tokens": 100, "iterations": [{"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 5, "cache_creation": {"ephemeral_5m_input_tokens": 2}}, {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 4}]}}"#,
    r#"{
        "prompt_tokens": 11,
        "completion_tokens": 2,
        "total_tokens": 13,
        "prompt_tokens_details": {
            "text_tokens": 2,
            "cached_tokens": 0,
            "cache_creation_tokens": 9,
            "cache_write_tokens": 9,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null,
            "cache_creation_token_details": {"ephemeral_5m_input_tokens": 9, "ephemeral_1h_input_tokens": 0}
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 2,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 9,
        "cache_read_input_tokens": 0,
        "iterations": [
            {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 5, "cache_creation": {"ephemeral_5m_input_tokens": 2}},
            {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 4}
        ],
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::reported_thinking_capped(
    Scenario::ReportedThinkingCapped,
    r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": 14}}}"#,
    r#"{
        "prompt_tokens": 1,
        "completion_tokens": 10,
        "total_tokens": 11,
        "prompt_tokens_details": {
            "text_tokens": 1,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 10,
            "text_tokens": 0,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::estimated_thinking_local(
    Scenario::EstimatedThinkingLocal,
    r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "reasoning_content": "hello world", "estimate": 2}"#,
    r#"{
        "prompt_tokens": 1,
        "completion_tokens": 10,
        "total_tokens": 11,
        "prompt_tokens_details": {
            "text_tokens": 1,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 2,
            "text_tokens": 8,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::signed_thinking_without_count(
    Scenario::SignedThinkingWithoutCount,
    r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": [{"type": "thinking", "thinking": "x"}]}}"#,
    r#"{
        "prompt_tokens": 1,
        "completion_tokens": 10,
        "total_tokens": 11,
        "prompt_tokens_details": {
            "text_tokens": 1,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": null,
            "text_tokens": null,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null
    }"#
)]
#[case::tool_search_count_fallback(
    Scenario::ToolSearchCountFallback,
    r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": [{"type": "server_tool_use", "name": "tool_search"}, {"type": "server_tool_use", "name": "x_tool_search_y"}, {"type": "server_tool_use", "name": "Tool_Search"}]}}"#,
    r#"{
        "prompt_tokens": 1,
        "completion_tokens": 10,
        "total_tokens": 11,
        "prompt_tokens_details": {
            "text_tokens": 1,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 10,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": null,
        "speed": null,
        "service_tier": null,
        "server_tool_use": {"web_search_requests": null, "tool_search_requests": 2, "browser_open_requests": null}
    }"#
)]
#[case::speed_tier_geography(
    Scenario::SpeedTierGeography,
    r#"{"usage": {"input_tokens": 1, "output_tokens": 2, "speed": "fast", "service_tier": "standard", "inference_geo": "us", "cache_read_input_tokens": null}, "speed": "slow"}"#,
    r#"{
        "prompt_tokens": 1,
        "completion_tokens": 2,
        "total_tokens": 3,
        "prompt_tokens_details": {
            "text_tokens": 1,
            "cached_tokens": 0,
            "cache_creation_tokens": 0,
            "cache_write_tokens": 0,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "completion_tokens_details": {
            "reasoning_tokens": 0,
            "text_tokens": 2,
            "accepted_prediction_tokens": null,
            "rejected_prediction_tokens": null,
            "audio_tokens": null,
            "image_tokens": null,
            "video_tokens": null
        },
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "iterations": null,
        "inference_geo": "us",
        "speed": "fast",
        "service_tier": "standard"
    }"#
)]
fn usage_cases(#[case] scenario: Scenario, #[case] input: &str, #[case] expected_json: &str) {
    let parsed = probe(input);
    let usage = project(input).unwrap();
    assert_eq!(as_json(&usage), expected(expected_json));
    match scenario {
        Scenario::CacheReadWriteTotal => cache_read_write_total_extras(),
        Scenario::FiveMinuteOneHourBreakdown => five_minute_one_hour_breakdown_extras(),
        Scenario::IterationUsageSum => iteration_usage_sum_extras(),
        Scenario::PartialIterationDetail => partial_iteration_detail_extras(),
        Scenario::ReportedThinkingCapped => reported_thinking_capped_extras(),
        Scenario::EstimatedThinkingLocal => estimated_thinking_local_extras(&parsed),
        Scenario::SignedThinkingWithoutCount => signed_thinking_without_count_extras(),
        Scenario::ToolSearchCountFallback => tool_search_count_fallback_extras(),
        Scenario::SpeedTierGeography => speed_tier_geography_extras(),
    }
}

fn cache_read_write_total_extras() {
    let falsey = project(
        r#"{"usage": {"input_tokens": null, "output_tokens": "abc", "cache_creation_input_tokens": null, "cache_read_input_tokens": null}}"#,
    )
    .unwrap();
    assert_eq!(falsey.prompt_tokens, 0);
    assert_eq!(falsey.completion_tokens, 0);
    assert_eq!(falsey.prompt_tokens_details.cached, 0);
    assert_eq!(falsey.prompt_tokens_details.cache_creation, 0);
    let zeroed = project(
        r#"{"usage": {"input_tokens": false, "output_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}"#,
    )
    .unwrap();
    assert_eq!(zeroed.completion_tokens, 0);
    assert_eq!(zeroed.cache_creation_input_tokens, 0);
    assert_eq!(zeroed.prompt_tokens_details.text, 0);
    let negative = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": -3}}"#,
    )
    .unwrap();
    assert_eq!(negative.cache_creation_input_tokens, -3);
    assert_eq!(negative.prompt_tokens, -2);
    assert_eq!(negative.prompt_tokens_details.text, 1);
}

fn five_minute_one_hour_breakdown_extras() {
    let partial = project(
        r#"{"usage": {"input_tokens": 10, "output_tokens": 5, "cache_creation": {"ephemeral_5m_input_tokens": 7}}}"#,
    )
    .unwrap();
    assert_eq!(
        partial.prompt_tokens_details.cache_creation_details,
        Some(CacheCreationTokenDetails {
            ephemeral_5m: Some(7),
            ephemeral_1h: None,
        })
    );
    let nonmapping =
        project(r#"{"usage": {"input_tokens": 10, "output_tokens": 5, "cache_creation": 5}}"#)
            .unwrap();
    assert!(
        nonmapping
            .prompt_tokens_details
            .cache_creation_details
            .is_none()
    );
    let total_only = project(
        r#"{"usage": {"input_tokens": 10, "output_tokens": 5, "cache_creation_input_tokens": 6}}"#,
    )
    .unwrap();
    assert!(
        total_only
            .prompt_tokens_details
            .cache_creation_details
            .is_none()
    );
    assert_eq!(total_only.prompt_tokens_details.text, 10);
    assert_eq!(total_only.prompt_tokens, 16);
    creation_listed_extras();
}

fn creation_listed_extras() {
    let listed =
        project(r#"{"usage": {"input_tokens": 10, "output_tokens": 5, "cache_creation": [7, 1]}}"#)
            .unwrap();
    assert!(
        listed
            .prompt_tokens_details
            .cache_creation_details
            .is_none()
    );
    let iteration_listed = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [{"input_tokens": 2, "output_tokens": 2, "cache_creation_input_tokens": 5, "cache_creation": [7, 1]}]}}"#,
    )
    .unwrap();
    assert!(
        iteration_listed
            .prompt_tokens_details
            .cache_creation_details
            .is_none()
    );
}

fn iteration_usage_sum_extras() {
    let empty = project(
        r#"{"usage": {"input_tokens": 100, "output_tokens": 7, "cache_creation_input_tokens": 9, "cache_read_input_tokens": 8, "iterations": []}}"#,
    )
    .unwrap();
    assert_eq!(empty.prompt_tokens, 117);
    assert_eq!(empty.completion_tokens, 7);
    assert_eq!(empty.iterations, Some(expected("[]")));
    let error = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [{"input_tokens": "4"}]}}"#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        UsageError::FieldType {
            field: "iterations.input_tokens"
        }
    ));
    let nonmapping = project(r#"{"usage": [10, 5, 3, 2]}"#).unwrap_err();
    assert!(matches!(nonmapping, UsageError::Decode { .. }));
    let listed_entry =
        project(r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [[5, 6]]}}"#)
            .unwrap_err();
    assert!(matches!(listed_entry, UsageError::Decode { .. }));
    iteration_container_extras();
}

fn iteration_container_extras() {
    let empty_containers = project(
        r#"{"usage": {"input_tokens": 9, "output_tokens": 9, "iterations": [{"input_tokens": [], "output_tokens": 2, "cache_read_input_tokens": {}}]}}"#,
    )
    .unwrap();
    assert_eq!(empty_containers.prompt_tokens, 0);
    assert_eq!(empty_containers.completion_tokens, 2);
    let listed_counts = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [{"input_tokens": [1], "output_tokens": 1}]}}"#,
    )
    .unwrap_err();
    assert!(matches!(
        listed_counts,
        UsageError::FieldType {
            field: "iterations.input_tokens"
        }
    ));
}

fn partial_iteration_detail_extras() {
    let over = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [{"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 1, "cache_creation": {"ephemeral_5m_input_tokens": 5, "ephemeral_1h_input_tokens": 2}}]}}"#,
    )
    .unwrap();
    assert_eq!(
        over.prompt_tokens_details.cache_creation_details,
        Some(CacheCreationTokenDetails {
            ephemeral_5m: Some(5),
            ephemeral_1h: Some(2),
        })
    );
    let fallback = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "cache_creation": {"ephemeral_5m_input_tokens": 3, "ephemeral_1h_input_tokens": 4}, "iterations": [{"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 2}]}}"#,
    )
    .unwrap();
    assert_eq!(
        fallback.prompt_tokens_details.cache_creation_details,
        Some(CacheCreationTokenDetails {
            ephemeral_5m: Some(3),
            ephemeral_1h: Some(4),
        })
    );
    assert_eq!(fallback.cache_creation_input_tokens, 2);
    coerced_text_extras();
}

fn coerced_text_extras() {
    let coerced = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 1, "iterations": [{"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 7, "cache_creation": {"ephemeral_5m_input_tokens": "", "ephemeral_1h_input_tokens": "7"}}]}}"#,
    )
    .unwrap();
    assert_eq!(
        coerced.prompt_tokens_details.cache_creation_details,
        Some(CacheCreationTokenDetails {
            ephemeral_5m: Some(0),
            ephemeral_1h: Some(7),
        })
    );
}

fn completion_split(source: &str) -> (Option<i64>, Option<i64>) {
    let usage = project(source).unwrap();
    (
        usage.completion_tokens_details.reasoning,
        usage.completion_tokens_details.text,
    )
}

fn reported_thinking_capped_extras() {
    thinking_clamp_extras();
    thinking_iteration_extras();
}

fn thinking_clamp_extras() {
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": 14}}}"#
        ),
        (Some(10), Some(0))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": -4}}}"#
        ),
        (Some(0), Some(10))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": "x"}}}"#
        ),
        (Some(0), Some(10))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": 5}}"#
        ),
        (Some(0), Some(10))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": [3]}}"#
        ),
        (Some(0), Some(10))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": -5, "output_tokens_details": {"thinking_tokens": 3}}}"#
        ),
        (Some(-5), Some(0))
    );
}

fn thinking_iteration_extras() {
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": 1}, "iterations": [{"input_tokens": 1, "output_tokens": 5, "output_tokens_details": {"thinking_tokens": 2}}, {"input_tokens": 1, "output_tokens": 5, "output_tokens_details": {"thinking_tokens": 3}}]}}"#
        ),
        (Some(5), Some(5))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": 4}, "iterations": [{"input_tokens": 1, "output_tokens": 5, "output_tokens_details": {"thinking_tokens": 2}}, {"input_tokens": 1, "output_tokens": 5}]}}"#
        ),
        (Some(4), Some(6))
    );
}

fn estimated_thinking_local_extras(probe: &Probe) {
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "output_tokens_details": {"thinking_tokens": 4}}, "reasoning_content": "hello world", "estimate": 9}"#
        ),
        (Some(4), Some(6))
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "reasoning_content": "", "completion_response": {"content": [{"type": "thinking"}]}, "estimate": 9}"#
        ),
        (None, None)
    );
    assert_eq!(
        completion_split(
            r#"{"usage": {"input_tokens": 1, "output_tokens": -5}, "reasoning_content": "hello world", "estimate": 2}"#
        ),
        (Some(0), Some(-5))
    );
    let over = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "reasoning_content": "hello world", "estimate": 20}"#,
    )
    .unwrap();
    assert_eq!(over.completion_tokens_details.reasoning, Some(10));
    assert_eq!(over.completion_tokens_details.text, Some(0));
    let failed = calculate_usage(
        &probe.usage,
        probe.reasoning_content.as_deref(),
        probe.completion_response.as_ref(),
        probe.speed.as_deref(),
        &FailingCounter,
    );
    assert!(matches!(failed, Err(UsageError::Counter { .. })));
}

fn signed_thinking_without_count_extras() {
    let redacted = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": [{"type": "redacted_thinking", "data": "x"}]}}"#,
    )
    .unwrap();
    assert_eq!(redacted.completion_tokens_details.reasoning, None);
    assert_eq!(redacted.completion_tokens_details.text, None);
    let text_only = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": [{"type": "text", "text": "x"}]}}"#,
    )
    .unwrap();
    assert_eq!(text_only.completion_tokens_details.reasoning, Some(0));
    assert_eq!(text_only.completion_tokens_details.text, Some(10));
    let empty = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": []}}"#,
    )
    .unwrap();
    assert_eq!(empty.completion_tokens_details.reasoning, Some(0));
    assert_eq!(empty.completion_tokens_details.text, Some(10));
    let nonlist = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": "text"}}"#,
    )
    .unwrap();
    assert_eq!(nonlist.completion_tokens_details.reasoning, Some(0));
    assert_eq!(nonlist.completion_tokens_details.text, Some(10));
}

fn tool_search_count_fallback_extras() {
    tool_search_reported_extras();
    tool_search_scan_extras();
}

fn tool_search_reported_extras() {
    let web_zero = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "server_tool_use": {"web_search_requests": 0}}, "completion_response": {"content": [{"type": "server_tool_use", "name": "tool_search"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        web_zero.server_tool_use,
        Some(ServerToolUse {
            web_search: Some(0),
            tool_search: Some(1),
            browser_open: None,
        })
    );
    let explicit_zero = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "server_tool_use": {"tool_search_requests": 0}}, "completion_response": {"content": [{"type": "server_tool_use", "name": "tool_search"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        explicit_zero.server_tool_use,
        Some(ServerToolUse {
            web_search: None,
            tool_search: Some(0),
            browser_open: None,
        })
    );
    let explicit = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "server_tool_use": {"web_search_requests": 2, "tool_search_requests": 3}}}"#,
    )
    .unwrap();
    assert_eq!(
        explicit.server_tool_use,
        Some(ServerToolUse {
            web_search: Some(2),
            tool_search: Some(3),
            browser_open: None,
        })
    );
}

fn tool_search_scan_extras() {
    let none = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10}, "completion_response": {"content": [{"type": "text", "text": "x"}]}}"#,
    )
    .unwrap();
    assert_eq!(none.server_tool_use, None);
    let listed = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 10, "server_tool_use": [1, 2]}}"#,
    )
    .unwrap();
    assert_eq!(listed.server_tool_use, None);
}

fn speed_tier_geography_extras() {
    let empty = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 2, "speed": ""}, "speed": "slow"}"#,
    )
    .unwrap();
    assert_eq!(empty.speed.as_deref(), Some(""));
    let absent = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 2, "service_tier": null}, "speed": "slow"}"#,
    )
    .unwrap();
    assert_eq!(absent.speed.as_deref(), Some("slow"));
    assert_eq!(absent.service_tier, None);
    let nonstring = project(
        r#"{"usage": {"input_tokens": 1, "output_tokens": 2, "speed": 5}, "speed": "slow"}"#,
    )
    .unwrap();
    assert_eq!(nonstring.speed.as_deref(), Some("slow"));
    assert!(is_anthropic_usage_object(&expected(
        r#"{"input_tokens": 1, "cache_creation_input_tokens": null}"#
    )));
    assert!(!is_anthropic_usage_object(&expected(
        r#"{"input_tokens": 1}"#
    )));
    assert!(!is_anthropic_usage_object(&expected(
        r#"{"input_tokens": 1, "prompt_tokens": 1, "cache_read_input_tokens": 0}"#
    )));
    assert!(!is_anthropic_usage_object(&expected(
        r#"{"cache_read_input_tokens": 0}"#
    )));
    assert!(!is_anthropic_usage_object(&expected("[]")));
}

#[rstest]
#[case::integer(r#"{"output_tokens_details": {"thinking_tokens": 7}}"#, Some(7))]
#[case::zero(r#"{"output_tokens_details": {"thinking_tokens": 0}}"#, Some(0))]
#[case::negative(r#"{"output_tokens_details": {"thinking_tokens": -3}}"#, Some(-3))]
#[case::boolean(r#"{"output_tokens_details": {"thinking_tokens": true}}"#, Some(1))]
#[case::float(r#"{"output_tokens_details": {"thinking_tokens": 2.5}}"#, None)]
#[case::text(r#"{"output_tokens_details": {"thinking_tokens": "4"}}"#, None)]
#[case::null(r#"{"output_tokens_details": {"thinking_tokens": null}}"#, None)]
#[case::missing_count(r#"{"output_tokens_details": {}}"#, None)]
#[case::details_not_object(r#"{"output_tokens_details": 5}"#, None)]
#[case::no_details(r#"{"output_tokens": 5}"#, None)]
#[case::usage_not_object("[]", None)]
fn thinking_tokens_from_usage_reads_only_integer_literals(
    #[case] usage: &str,
    #[case] want: Option<i64>,
) {
    assert_eq!(thinking_tokens_from_usage(&expected(usage)), want);
}
