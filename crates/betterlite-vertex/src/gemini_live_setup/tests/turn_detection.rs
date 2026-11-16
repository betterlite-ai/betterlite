#![cfg(test)]

use rstest::rstest;

use betterlite_prompts::arguments::json::Json;

use crate::gemini_live_setup::turn_detection::map_automatic_turn_detection;

use super::support::{as_json, json, turn_detection};

#[rstest]
#[case::semantic_without_override(r#"{"type": "semantic_vad"}"#, None)]
#[case::semantic_guardrail(
    r#"{"type": "semantic_vad", "create_response": false}"#,
    Some(r#"{"disabled": true}"#)
)]
#[case::semantic_enabled(
    r#"{"type": "semantic_vad", "create_response": true}"#,
    Some(r#"{"disabled": false}"#)
)]
#[case::server_vad(r#"{"type": "server_vad"}"#, Some(r#"{"disabled": false}"#))]
#[case::server_vad_guardrail(
    r#"{"type": "server_vad", "create_response": false, "prefix_padding_ms": 20, "silence_duration_ms": 500}"#,
    Some(r#"{"disabled": true, "prefixPaddingMs": 20, "silenceDurationMs": 500}"#)
)]
#[case::unknown_kind(r#"{"type": "custom_vad"}"#, Some(r#"{"disabled": true}"#))]
#[case::no_kind(r#"{}"#, Some(r#"{"disabled": true}"#))]
#[case::guardrail_without_kind(r#"{"create_response": false}"#, Some(r#"{"disabled": true}"#))]
#[case::fractional_durations(
    r#"{"type": "server_vad", "prefix_padding_ms": 20.5, "silence_duration_ms": 500.0}"#,
    Some(r#"{"disabled": false}"#)
)]
#[case::boolean_duration(
    r#"{"type": "server_vad", "silence_duration_ms": true}"#,
    Some(r#"{"disabled": false, "silenceDurationMs": true}"#)
)]
#[case::text_duration(
    r#"{"type": "server_vad", "prefix_padding_ms": "20", "silence_duration_ms": 500}"#,
    Some(r#"{"disabled": false, "silenceDurationMs": 500}"#)
)]
#[case::negative_duration(
    r#"{"type": "server_vad", "prefix_padding_ms": -5}"#,
    Some(r#"{"disabled": false, "prefixPaddingMs": -5}"#)
)]
#[case::out_of_range_duration(
    r#"{"type": "server_vad", "prefix_padding_ms": 18446744073709551616}"#,
    Some(r#"{"disabled": false}"#)
)]
fn automatic_detection_follows_the_override_rules(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let detection = map_automatic_turn_detection(&turn_detection(source));
    let expected = expected.map_or(Json::Null, json);
    assert_eq!(as_json(&detection), expected);
}
