#![cfg(test)]

use rstest::rstest;

use serde_json::Value;

use super::promote_message_usage;

#[rstest]
#[case::start_cache_into_delta(r#"[{"type":"message_start","message":{"usage":{"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"cache_creation":{"ephemeral_5m_input_tokens":5}}}},{"type":"message_delta","usage":{}}]"#, r#"[{"type":"message_start","message":{"usage":{"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"cache_creation":{"ephemeral_5m_input_tokens":5}}}},{"type":"message_delta","usage":{"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"cache_creation":{"ephemeral_5m_input_tokens":5}}}]"#)]
#[case::stop_metrics_promoted(r#"[{"type":"message_delta","usage":{"keep":1},"amazon-bedrock-invocationMetrics":{"inputTokenCount":2}},{"type":"message_stop","usage":{"input_tokens":2}}]"#, r#"[{"type":"message_delta","usage":{"keep":1,"input_tokens":2}},{"type":"message_stop","usage":{"input_tokens":2}}]"#)]
#[case::existing_delta_usage(r#"[{"type":"message_delta","usage":{"keep":1}},{"type":"message_delta","usage":{"cache_read_input_tokens":2}},{"type":"message_stop","usage":{"cache_creation_input_tokens":3,"input_tokens":4}}]"#, r#"[{"type":"message_delta","usage":{"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"input_tokens":4}},{"type":"message_stop","usage":{"cache_creation_input_tokens":3,"input_tokens":4}}]"#)]
#[case::missing_stop_metrics(r#"[{"type":"message_start","message":{"usage":{"cache_read_input_tokens":4}}},{"type":"message_delta","usage":{}}]"#, r#"[{"type":"message_start","message":{"usage":{"cache_read_input_tokens":4}}},{"type":"message_delta","usage":{"cache_read_input_tokens":4}}]"#)]
#[case::cache_creation_details(r#"[{"type":"message_start","message":{"usage":{"cache_creation":{"ephemeral_5m_input_tokens":5}}}},{"type":"message_delta","usage":{"cache_creation":null}},{"type":"message_stop","usage":{"cache_creation":{"ignored":1}}}]"#, r#"[{"type":"message_start","message":{"usage":{"cache_creation":{"ephemeral_5m_input_tokens":5}}}},{"type":"message_delta","usage":{"cache_creation":null}},{"type":"message_stop","usage":{"cache_creation":{"ignored":1}}}]"#)]
#[case::event_order_after_promotion(r#"[{"type":"message_stop","usage":{"input_tokens":99}},{"type":"message_delta","usage":{}},{"type":"message_stop","usage":{}},{"type":"other","value":1}]"#, r#"[{"type":"message_stop","usage":{"input_tokens":99}},{"type":"message_delta","usage":{}},{"type":"message_stop","usage":{}},{"type":"other","value":1}]"#)]
fn message_usage_cases(#[case] input: &str, #[case] expected: &str) {
    let events: Vec<Value> = serde_json::from_str(input).unwrap();
    let wanted: Vec<Value> = serde_json::from_str(expected).unwrap();
    assert_eq!(promote_message_usage(events), wanted);
}
